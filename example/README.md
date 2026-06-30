# Example crate — a trivial target for `cargo-testmap`

This is a small, intentionally-simple Rust library whose only job is to give
`cargo-testmap` something interesting to map: a handful of functions, each
exercised by a mix of unit tests (inline `#[cfg(test)]` modules) and an
integration test (`tests/integration.rs`).

That mix is what makes the *reverse* map non-trivial:

- some lines are covered by exactly **one** test,
- some lines are covered by **several** tests (unit + integration hitting the
  same code path),
- some lines are **not covered at all**.

## Layout

```
example/
├── Cargo.toml
├── src/
│   ├── lib.rs        # public API + module wiring
│   ├── calc.rs       # arithmetic, with branches
│   └── parser.rs     # tiny tokenizer/parser
└── tests/
    └── integration.rs  # end-to-end tests (cover cross-module code paths)
```

## Usage

From anywhere inside this directory:

```sh
cargo testmap collect --lib --tests
cargo testmap report --theme "base16-mocha.dark"
# open ../target/testmap/report/index.html
```
