//! One asynchronous WinDNS operation retained through its actual native callback.
use super::{Completion, Identity, NetworkError, PendingDns};
use std::{
    cell::UnsafeCell,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ptr,
    sync::{Arc, Condvar, Mutex},
};
use windows_sys::Win32::{Foundation::DNS_REQUEST_PENDING, NetworkManagement::Dns::*};

struct Submission {
    started: bool,
    returned: bool,
    pending: bool,
}

pub(super) struct NativeQuery {
    // These allocations never move while WinDNS owns their pointers. WinDNS is the
    // only writer until immediate return or the one completion callback.
    request: UnsafeCell<DNS_QUERY_REQUEST>,
    result: UnsafeCell<DNS_QUERY_RESULT>,
    cancel: UnsafeCell<DNS_QUERY_CANCEL>,
    submitted: Mutex<Submission>,
    submitted_changed: Condvar,
    completion: Mutex<Option<Completion>>,
    addresses: Mutex<Result<Vec<IpAddr>, NetworkError>>,
    query: Arc<PendingDns>,
}

// WinDNS accesses only its stable request/result/cancel storage. Rust accesses
// results only at actual completion; submission and completion state are locked.
unsafe impl Send for NativeQuery {}
unsafe impl Sync for NativeQuery {}

impl NativeQuery {
    pub(super) fn prepare(
        name: &str,
        identity: Identity,
        query_type: u16,
    ) -> Result<Arc<Self>, NetworkError> {
        if identity.index == 0
            || identity.luid == 0
            || name.is_empty()
            || name.len() > 253
            || name.contains('\0')
            || !matches!(query_type, DNS_TYPE_A | DNS_TYPE_AAAA)
        {
            return Err(NetworkError::Dns);
        }
        // The cancellation closure is populated before this query is published.
        // A weak reference avoids a query -> cancel -> native query ownership cycle.
        let slot = Arc::new(Mutex::new(std::sync::Weak::<Self>::new()));
        let cancel_slot = Arc::clone(&slot);
        let (query, completion) = PendingDns::with_cancel(name, move || {
            if let Ok(slot) = cancel_slot.lock() {
                let native = slot.upgrade();
                drop(slot);
                if let Some(native) = native {
                    native.cancel_pending();
                }
            }
        });
        let native = Arc::new(Self {
            request: UnsafeCell::new(DNS_QUERY_REQUEST {
                Version: DNS_QUERY_REQUEST_VERSION1,
                QueryName: query.name.as_ptr(),
                QueryType: query_type,
                QueryOptions: u64::from(DNS_QUERY_STANDARD),
                InterfaceIndex: identity.index,
                pQueryCompletionCallback: Some(completed),
                ..Default::default()
            }),
            result: UnsafeCell::new(DNS_QUERY_RESULT {
                Version: DNS_QUERY_RESULTS_VERSION1,
                ..Default::default()
            }),
            cancel: UnsafeCell::new(DNS_QUERY_CANCEL::default()),
            submitted: Mutex::new(Submission {
                started: false,
                returned: false,
                pending: false,
            }),
            submitted_changed: Condvar::new(),
            completion: Mutex::new(Some(completion)),
            addresses: Mutex::new(Err(NetworkError::Dns)),
            query,
        });
        *slot.lock().map_err(|_| NetworkError::Dns)? = Arc::downgrade(&native);
        Ok(native)
    }

    pub(super) fn pending(&self) -> Arc<PendingDns> {
        Arc::clone(&self.query)
    }
    pub(super) async fn addresses(&self) -> Result<Vec<IpAddr>, NetworkError> {
        std::future::poll_fn(|cx| self.query.poll_complete(cx)).await?;
        self.addresses
            .lock()
            .map_err(|_| NetworkError::Dns)?
            .clone()
    }

    // Caller must register pending() with the attempt before submission. Keep
    // this Arc until the attempt drains, even if the resolver future is dropped.
    pub(super) fn submit(self: &Arc<Self>) -> Result<(), NetworkError> {
        {
            let mut submission = self.submitted.lock().map_err(|_| NetworkError::Dns)?;
            if submission.started || submission.returned {
                return Err(NetworkError::Dns);
            }
            self.query.begin_native()?;
            submission.started = true;
        }
        let raw = Arc::into_raw(Arc::clone(self));
        // SAFETY: raw owns a callback reference and all native buffers are stable.
        let status = unsafe {
            (*self.request.get()).pQueryContext = raw.cast_mut().cast();
            DnsQueryEx(self.request.get(), self.result.get(), self.cancel.get())
        };
        if status != DNS_REQUEST_PENDING {
            // Immediate completion does not schedule a callback. Reclaim exactly
            // the reference that would otherwise belong to that callback.
            unsafe {
                drop(Arc::from_raw(raw));
            }
            if status != 0 {
                unsafe {
                    (*self.result.get()).QueryStatus = status;
                }
            }
            self.complete_result();
        }
        let mut submission = self.submitted.lock().map_err(|_| NetworkError::Dns)?;
        submission.pending = status == DNS_REQUEST_PENDING;
        submission.returned = true;
        self.submitted_changed.notify_all();
        drop(submission);
        self.query.end_native();
        Ok(())
    }

    fn cancel_pending(&self) {
        let Ok(mut submission) = self.submitted.lock() else {
            return;
        };
        if !submission.started {
            submission.returned = true;
            self.submitted_changed.notify_all();
            drop(submission);
            self.complete_result();
            return;
        }
        // A cancellation arriving while submission is in flight must not use an
        // uninitialized cancel handle. It waits for submission, not DNS completion.
        while !submission.returned {
            let Ok(next) = self.submitted_changed.wait(submission) else {
                return;
            };
            submission = next;
        }
        let pending = submission.pending;
        drop(submission);
        if pending {
            // No state lock crosses this call: its callback may run before return.
            unsafe {
                DnsCancelQuery(self.cancel.get());
            }
        }
    }

    fn complete_result(&self) {
        let Ok(mut completion) = self.completion.lock() else {
            return;
        };
        let Some(completion) = completion.take() else {
            return;
        };
        // Native has finished writing. The callback result owns the whole list,
        // including CNAME/other records that are ignored by address extraction.
        let result = unsafe { &mut *self.result.get() };
        let records = result.pQueryRecords;
        let mut addresses = Vec::new();
        if result.QueryStatus == 0 {
            let mut record = records;
            while !record.is_null() {
                let item = unsafe { &*record };
                match item.wType {
                    DNS_TYPE_A => addresses.push(IpAddr::V4(Ipv4Addr::from(
                        unsafe { item.Data.A.IpAddress }.to_ne_bytes(),
                    ))),
                    DNS_TYPE_AAAA => addresses.push(IpAddr::V6(Ipv6Addr::from(unsafe {
                        item.Data.AAAA.Ip6Address.IP6Byte
                    }))),
                    _ => {}
                }
                record = item.pNext;
            }
        }
        if let Ok(mut output) = self.addresses.lock() {
            *output = if result.QueryStatus == 0 {
                Ok(addresses)
            } else {
                Err(NetworkError::Dns)
            };
        }
        result.pQueryRecords = ptr::null_mut();
        completion.complete(|| {
            if !records.is_null() {
                unsafe {
                    DnsFree(records.cast(), DnsFreeRecordList);
                }
            }
        });
    }
}

unsafe extern "system" fn completed(
    context: *const core::ffi::c_void,
    _result: *mut DNS_QUERY_RESULT,
) {
    // Exactly one callback consumes the native-owned Arc after a pending result.
    let query = unsafe { Arc::from_raw(context.cast::<NativeQuery>()) };
    query.complete_result();
}
