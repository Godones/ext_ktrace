# tp-lexer: no_std filter expression engine for tracepoint-like events

`tp-lexer` parses and evaluates boolean filter expressions combining numeric and byte/string comparisons for event pipelines (eBPF / tracepoints / custom telemetry). It targets `no_std` (with `alloc`) and supports direct evaluation over raw byte buffers or map-based contexts.

## Features
- Numeric operators: `== != < <= > >=` (bit mask form: `field & 0x10 == 0x10`).
- Byte/string operators: `== != ~` (glob: `* ? [..]` with class negation `[!a]` / `[^a]`).
- Unquoted barewords allowed: `comm != bash`; quote when spaces or special chars present (`"C Program"`).
- Precedence: parentheses > comparison > `&&` > `||` (short-circuit evaluation).
- Tri-state runtime: missing field => `Unknown`.
  - `Unknown && True` => `True` (true-neutral)
  - `Unknown || False` => `False` (false-neutral)
  - Top-level `Unknown` => treated as `True` (fail-open to avoid silent data loss).
- Signed decimal / hex integer literals such as `-1`, `0x10`, and `-(0x10)`.
- `u64` / `u128` / `i128` / `isize` / `usize` fields, plus their `NonZero*` variants, are
  supported, but evaluation still uses an internal `i64`; values outside the `i64` range are
  truncated with Rust's integer cast semantics before comparison.
- Compile-time validation: unknown field or type mismatch produces a `ParseError`.
- Raw buffer evaluation (`BufContext`) using schema offsets; no copying.

## Schema Definition
Define schemas with the `schema!` macro. Each entry: `"name" => (FieldType, offset, length)`.

Integer types implement the `FieldClassifier` trait exposing `FIELD_TYPE`. Bytes fields use `FieldType::Bytes`.

```rust
use tp_lexer::{schema, FieldType, compile_with_schema};

let schema = schema!(
    "sig"  => (u32::FIELD_TYPE, 0, 4),      // u32 at bytes 0..4
    "comm" => (FieldType::Bytes, 4, 12),    // 12 bytes at 4..16
);
let expr = "sig >= 10 && comm != bash";
let compiled = compile_with_schema(expr, schema).unwrap();
```

## Map-Based Context Example

```rust
use alloc::collections::BTreeMap;
use tp_lexer::{schema, FieldType, compile_with_schema};

let schema = schema!(
    "sig"  => (u32::FIELD_TYPE, 0, 4),
    "comm" => (FieldType::Bytes, 4, 16),
);
let compiled = compile_with_schema("sig >= 10 && comm != bash", schema).unwrap();
let mut ctx = BTreeMap::from([
    ("sig".to_string(), "12".to_string()),
    ("comm".to_string(), "sh".to_string()),
]);
assert!(compiled.evaluate(&ctx));
```

## Raw Buffer Context Example

```rust
use tp_lexer::{schema, FieldType, compile_with_schema, BufContext};

let schema = schema!(
    "pid"  => (u32::FIELD_TYPE, 0, 4),
    "comm" => (FieldType::Bytes, 4, 16),
);
let mut buf = [0u8; 32];
buf[0..4].copy_from_slice(&1234u32.to_le_bytes());
buf[4..8].copy_from_slice(b"bash");
let ctx = BufContext::new(&buf, &schema);
let compiled = compile_with_schema("pid == 1234 && comm == bash", schema).unwrap();
assert!(compiled.evaluate(&ctx));
```

## Glob Matching

```rust
let schema = schema!("comm" => (FieldType::Bytes, 0, 16));
let compiled = compile_with_schema("comm ~ bash*", schema).unwrap();
let mut buf = [0u8; 32];
buf[0..4].copy_from_slice(&0u32.to_le_bytes()); // unused
buf[4..8].copy_from_slice(b"bash");
let ctx = BufContext::new(&buf, &schema);
assert!(compiled.evaluate(&ctx));
```

## Bit Mask Example

```rust
let schema = schema!("flags" => (u32::FIELD_TYPE, 0, 4));
let compiled = compile_with_schema("flags & 0x10 == 0x10", schema).unwrap();
let mut buf = [0u8; 8];
buf[0..4].copy_from_slice(&0x30u32.to_le_bytes()); // 0x30 & 0x10 == 0x10 -> true
let ctx = BufContext::new(&buf, &schema);
assert!(compiled.evaluate(&ctx));
```

## Errors & Semantics
Compile-time errors:
- Unknown field names
- Type mismatches (numeric op on bytes field; string op on integer field)
- Syntax errors (unrecognized tokens, unterminated string, lone operator characters)

Runtime semantics:
- Missing values => `Unknown` tri-state propagation
- Top-level `Unknown` => treated as `true` (fail-open)
- Short-circuit evaluation limits unnecessary field access
- Wide integer fields (`u64` / `u128` / `i128` / `isize` / `usize`) and their `NonZero*`
  variants are cast to the engine's internal `i64` representation before comparison, which may
  truncate high bits or flip sign for out-of-range values

## Performance Notes
- Bit masking done after integer fetch; consider pre-masked fields if hot.
- Glob matcher is a simple recursive backtracking implementation; patterns dominated by leading `*` may degrade performance—structure patterns to avoid pathological worst cases.
- For high-throughput paths, prefer `BufContext` to avoid string allocations.

## Non-Goals / Current Omissions
- No regex (glob only)
- No arithmetic besides bit masking
- No user-defined functions
- No range syntax (`[a,b]`) or set membership; can be emulated with chained comparisons / ORs

## Testing
Operator coverage tests ensure every numeric and string comparison path, mask usage, and type mismatch branch is exercised (`cargo test -p tp-lexer`).

## Build & Run
```zsh
cargo test -p tp-lexer
cargo run  -p tp-lexer -- 'comm ~ bash* && (sig == 17 || sig >= 10)'
```

## Tri-State Rationale
Fail-open semantics (Unknown => true) prevent accidental data loss when filters reference newly introduced fields not yet populated. If strict filtering is required, introduce an explicit presence indicator field and include it in the expression.

## Reference
- [Linux tracepoint filter docs](https://www.kernel.org/doc/html/v4.17/trace/events.html)
- [The implementation of tracepoint filters in the Linux kernel](https://www.kernel.org/doc/html/v4.17/trace/events.html)
