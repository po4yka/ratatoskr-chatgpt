# ChatGPT archive requirements

## Goals

1. Preserve every official user-provided export immutably before normalization.
2. Version parsers and retain unknown records for forward compatibility.
3. Reconstruct projects, conversation graphs, message revisions/content parts, files, Canvas, and available assets.
4. Report completeness and limitations rather than claiming unsupported restoration fidelity.
5. Support portable local export and optional authorized Compliance ingestion.

## Non-goals

OpenAI inference, treating Responses/API Conversations as ChatGPT history, consumer browser login/cookies, or claiming imports can restore the original ChatGPT UI/workspace.

## Requirements

- Raw archive hash/blob and import metadata are durable before parsing.
- Archive extraction is bounded and path-safe.
- Conversations are graphs, not only ordered lists.
- Unknown record/content variants are preserved.
- New snapshots/revisions do not overwrite prior evidence.
- Missing data does not prove deletion; completeness status is explicit.
- Knowledge receives authorized normalized references, not archive authority.

First slice: synthetic official-export fixture -> raw blob -> versioned parser -> one project/conversation graph -> completeness report -> portable JSON/Markdown.
