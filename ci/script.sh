set -ex

export CARGO_INCREMENTAL=0

if [ "$CLIPPY" = "yes" ]; then
      cargo clippy --all -- -D warnings
elif [ "$DOCS" = "yes" ]; then
    cargo clean
    cargo doc --all --no-deps
    cd book
    mdbook build
    cd ..
    cp -r book/book/html/ target/doc/book/
    travis-cargo doc-upload || true
elif [ "$RUSTFMT" = "yes" ]; then
    cargo fmt --all -- --check
elif [ "$INTEGRATION_TESTS" = "yes" ]; then
    cargo build
    cd integration_tests
    if [ "$GNUPLOT" = "yes" ]; then
        cargo test -- --format=pretty --nocapture --ignored
    fi
    cargo test -- --format=pretty --nocapture

else
    export RUSTFLAGS="-D warnings"

    cargo check --no-default-features --features gnuplot_backend
    cargo check --no-default-features --features plotters_backend

    cargo check --all-features

    cargo test   
fi
