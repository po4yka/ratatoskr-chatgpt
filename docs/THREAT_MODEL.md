# ChatGPT archive threat model

## Assets

Conversations, projects/instructions, uploaded/generated files, account metadata, raw archives, Compliance credentials, normalized graph, and portable exports.

## Threats and controls

- **Zip bomb/path traversal/symlink:** file/count/size/decompression/path limits, isolated extraction, content-addressed destinations.
- **Malicious HTML/file/content:** no execution/active rendering; MIME sniff; sandbox viewers; output escaping.
- **Parser confusion/data loss:** explicit schema detection/version, raw preservation, unknown records, completeness warnings.
- **Cross-user disclosure:** owner authorization at archive/blob/query/export boundaries.
- **Telemetry leak:** no message text, prompts, filenames, or raw errors in logs/metrics.
- **Compliance credential/cursor compromise:** encrypted least-privilege secrets, audit, bounded pull, rotation/revoke.
- **False deletion/completeness:** conservative snapshot semantics and evidence-based status.
- **Portable export leakage:** explicit authorization, safe paths, deterministic manifest, protected delivery.

Re-review for rendering Canvas/HTML, remote file fetch, organization exports, sharing/collaboration, or automatic delete synchronization.
