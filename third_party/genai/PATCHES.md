# Local patch to genai 0.2.4

Upstream: https://github.com/jeremychone/rust-genai (MIT OR Apache-2.0, see LICENSE files).

`src/adapter/adapters/openai/streamer.rs`: a streamed chunk whose `delta` has no
`content` (providers that stream `reasoning`, tool calls or an empty delta first,
for example GLM models on gateways) made `x_take(...)?` fail with
`PropertyNotFound("/delta/content")` and end the reply. A missing property is
now treated as "no content in this chunk".

`src/adapter/adapters/openai/streamer.rs` (second change): providers that close the
stream without the `[DONE]` message made the event source report `StreamEnded`, which
was returned as an error ("The reply was cut off") after the full text had arrived.
It now ends the reply normally.

`src/adapter/adapters/gemini/adapter_impl.rs`: the Gemini API key was appended to the
request URL (`?key=...`), which leaks it into logs and error text. It is now sent in the
`x-goog-api-key` header, which Gemini accepts.

Drop this folder and the `[patch.crates-io]` entry in the root Cargo.toml once the
pinned genai version handles it (newer releases do).
