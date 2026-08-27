# Owner fixture discovery and golden admission

Real ChatGPT exports are private evidence. They never enter this repository, a pull request,
ordinary logs, or an issue attachment. A fixture derived from real evidence becomes a parser golden
only through the owner-controlled process below. Passing this process does not by itself add parser
support: the detector, compiled parser, unknown-record preservation, completeness output, and golden
contract must land together.

## 1. Authorize and receive privately

The repository owner must explicitly authorize one export for parser discovery. In an
access-controlled record outside Git, record the purpose, acquisition mode, provider receive time,
access list, retention/disposition decision, and consent/license terms. Store the original export
only in the protected archive service or another owner-controlled private location. Record its
immutable digest in that private record; do not copy the digest, account identity, filenames, chat
titles, messages, or asset bytes into the candidate or ordinary command output.

Use the production receipt boundary so the original passes the same streaming hash and hostile ZIP
inspection as any other archive:

```bash
curl -X POST http://127.0.0.1:9084/exports \
  -H 'Authorization: Bearer <owner-provided-local-token>' \
  -H 'X-Ratatoskr-Acquisition: consumer_export' \
  -H 'Content-Type: application/zip' \
  --data-binary @<private-export-path>
```

The token and private path are shell placeholders and must never be committed or pasted into logs.
If production inspection rejects or quarantines the archive, stop. Do not weaken intake limits or
unzip the export into the checkout.

## 2. Discover structure in private storage

Work only from an owner-controlled private workspace. Record the detector signals, acquisition mode,
schema decision, record-variant inventory, graph relationship shape, unknown variants, completeness,
and missing-asset categories in the private evidence record. Do not record content values. A new or
incompatible shape receives a new private discovery record; it is not silently added to an existing
parser declaration.

## 3. Minimize and synthesize

Build the smallest JSON case that preserves the decision-relevant structure. Replace every external
identifier, timestamp when it is not structurally relevant, title, prompt, response, filename, URL,
account reference, and asset with deterministic synthetic values. Remove unrelated records. Preserve
ordering, graph edges, and unknown variants only where the parser decision depends on them.

The candidate directory has exactly these human-authored inputs:

```text
candidate/
  manifest.json
  derived.json
  observed-structure.json
  expected-structure.json
```

`observed-structure.json` is the minimized structural observation and
`expected-structure.json` is the reviewed expectation. They must be canonically equal at admission.
`manifest.json` uses the strict format exercised by
`crates/chatgpt-archive/tests/golden_fixture_contract.rs`. Its
`private_evidence_record` is a non-sensitive opaque reference, never a source digest.

## 4. Compare and review

In the private workspace, compare the source and derived case through the same detector/parser code.
The schema signals, exact parser selection, record-variant inventory, relationship shape, unknown
preservation, and completeness class must match. Then complete all manifest review gates: consent,
license, secret scan, personal-data scan, hostile-path review, deterministic comparison, independent
review, and explicit owner approval.

Run the read-only admission gate from the repository root:

```bash
build-gate -- cargo run -p ratatoskr-chatgpt-archive-service -- \
  fixture-admit --candidate <private-derived-candidate-directory>
```

Exit 0 with `status: admitted` permits review; exit 1 refuses the candidate. The command never copies,
rewrites, or blesses files. A raw archive, symlink, unsafe manifest path, secret or personal marker,
unknown manifest field, missing review, or structural mismatch is a refusal.

## 5. Admit and bless explicitly

After approval, copy only the minimized candidate into
`crates/chatgpt-archive/tests/golden/owner-derived/<case-id>/`. Add or update a read-only parser golden
test. Golden updates are normal reviewed diffs; tests and admission never rewrite them. Inspect every
line before commit, then run:

```bash
build-gate -- cargo test --locked -p ratatoskr-chatgpt-archive --test golden_fixture_contract
```

Do not broaden the real provider support matrix until a compiled parser golden also proves detector
selection, unknown preservation, and conservative completeness for the admitted shape. The synthetic
example currently committed documents the gate only and makes no real ChatGPT export support claim.

## 6. Dispose and audit privately

Record the admitted commit, parser/schema decision, reviewers, and final disposition in the private
evidence record. Delete temporary derived workspaces according to that record. Retain or delete the
original only under the owner's archive/retention decision; repository cleanup is not authority to
erase preserved archive evidence.
