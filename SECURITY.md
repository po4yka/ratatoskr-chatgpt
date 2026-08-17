# Security Policy for Ratatoskr ChatGPT Archive

Report vulnerabilities privately. Do not publish personal exports, conversations, files, generated images, account metadata, Compliance credentials, or production parser failures containing source data.

Security review is required for upload/archive intake, decompression, paths, MIME, HTML rendering, files/assets, parser changes, Compliance ingestion, BlobStore access, portable export, retention/deletion, and logs.

Baseline: no consumer browser login/session automation; immutable raw archive first; path/file/count/size/decompression limits; no execution or active HTML; content-addressed names; owner authorization; encrypt credentials; preserve unknown data safely; redact source text/filenames from telemetry; treat all message/tool content as hostile data.
