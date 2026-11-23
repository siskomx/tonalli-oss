package org.tonalli.spike.saf

import java.io.IOException

data class SafBindingRequest(val dataset: String, val manifestHash: String, val revision: String,
    val fingerprint: String, val receiptHash: String) {
    companion object {
        const val ACTION = "org.tonalli.spike.SAF_BENCHMARK"
        const val BIND = "org.tonalli.spike.SAF_BENCHMARK_BIND"
        fun checked(dataset: String?, hash: String?, revision: String?, fingerprint: String?, receipt: String?): SafBindingRequest {
            if (dataset == null || !Regex("[a-z][a-z0-9_-]{0,31}").matches(dataset) ||
                hash == null || !Regex("[a-f0-9]{64}").matches(hash) ||
                revision == null || !Regex("[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}").matches(revision) ||
                fingerprint == null || !Regex("[a-f0-9]{64}").matches(fingerprint) ||
                receipt == null || !Regex("[a-f0-9]{64}").matches(receipt)) throw IOException("Invalid SAF benchmark binding request")
            return SafBindingRequest(dataset, hash, revision, fingerprint, receipt)
        }
    }
}
