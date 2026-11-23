//! Tonalli modification: fail-closed Windows interface identity and socket binding.
use socket2::Socket;
use std::{io, mem, os::windows::io::AsRawSocket};
use windows_sys::Win32::{
    NetworkManagement::{
        IpHelper::{GetIfEntry2, GetIpInterfaceEntry, MIB_IF_ROW2, MIB_IPINTERFACE_ROW},
        Ndis::IfOperStatusUp,
    },
    Networking::WinSock::{
        setsockopt, WSAGetLastError, AF_INET, AF_INET6, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF,
        IP_UNICAST_IF, SOCKET_ERROR,
    },
};

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        "Captured Windows interface is unavailable",
    )
}

pub(super) fn bind(
    socket: &Socket,
    ipv6: bool,
    captured_index: u32,
    expected_luid: u64,
) -> io::Result<()> {
    if captured_index == 0 || expected_luid == 0 {
        return Err(invalid());
    }
    // These Win32 structures contain integers/unions only; zero initializes every
    // field before the documented identity/family inputs are supplied.
    let mut interface: MIB_IF_ROW2 = unsafe { mem::zeroed() };
    interface.InterfaceIndex = captured_index;
    // SAFETY: initialized writable structure, valid for this synchronous API call.
    let status = unsafe { GetIfEntry2(&mut interface) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    if unsafe { interface.InterfaceLuid.Value } != expected_luid
        || interface.OperStatus != IfOperStatusUp
    {
        return Err(invalid());
    }
    let mut family: MIB_IPINTERFACE_ROW = unsafe { mem::zeroed() };
    family.Family = if ipv6 { AF_INET6 } else { AF_INET };
    family.InterfaceLuid = interface.InterfaceLuid;
    // Resolve by LUID for the actual socket family; IPv4/IPv6 indices need not
    // be interchangeable. Never pass index zero to the socket as a fallback.
    let status = unsafe { GetIpInterfaceEntry(&mut family) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    if family.InterfaceIndex == 0
        || unsafe { family.InterfaceLuid.Value } != expected_luid
        || !family.Connected
    {
        return Err(invalid());
    }
    let (level, option, value) = if ipv6 {
        (IPPROTO_IPV6, IPV6_UNICAST_IF, family.InterfaceIndex)
    } else {
        (IPPROTO_IP, IP_UNICAST_IF, family.InterfaceIndex.to_be())
    };
    // SAFETY: live socket; DWORD points to four initialized bytes for the duration
    // of setsockopt. Windows specifies IPv4 network order and IPv6 host order.
    let status = unsafe {
        setsockopt(
            socket.as_raw_socket() as _,
            level,
            option,
            (&value as *const u32).cast(),
            mem::size_of::<u32>() as i32,
        )
    };
    if status == SOCKET_ERROR {
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    Ok(())
}
