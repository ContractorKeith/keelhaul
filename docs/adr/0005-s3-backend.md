# S3: conditional PUT, strict endpoints, errors that never leak

Governs `src/backend/s3.rs` and `src/backend/s3/`.

**Create-once.** Exact create-once needs conditional PUT (`If-None-Match: *`). At open, the backend probes support by PUTting an isolated temporary object twice and expecting the second to be refused, so servers that silently ignore the header are detected too; if the process dies before the DELETE, SPEC §13 lets prune remove the `.tmp-` object once it is older than 30 minutes. Without conditional PUT, writes fall back to HEAD-then-PUT. That is safe for content-addressed names, because racing writers of one name write identical bytes, and config and key slots are written once at init. Concurrent `init` of one repository on such a server is therefore unsafe; this was documented and accepted on 2026-11-04 rather than serialising every upload.

**Endpoints.** `S3Location::parse` uses the signer's own URL parser (`rusty_s3::Bucket`) instead of adding a second URL dependency. `Bucket` deliberately strips userinfo and query, so the original string is also checked for `?`, `#`, `\` and `@`. The bucket is the first path segment; endpoint path prefixes are not supported. The SigV4 signature covers the Host header, so the test fault proxy preserves it.

**Errors.** HTTP errors and response bodies are never retained: either can contain presigned URLs. Retry causes are fixed strings independent of URLs, credentials and server text, and S3 error XML is parsed for the machine-readable `<Code>` only. Credentials are private and `Debug` redacts them; a `compile_fail` doc test pins that.

**Ranges.** A server that ignores `Range` returns 200 and the full body, which is sliced locally; a 206 must carry a Content-Range that matches the clipped request.
