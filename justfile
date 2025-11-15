copy:
    repomix --stdout | pbcopy

build:
    cargo build

check:
    cargo check

test:
    cargo test

one:
    cargo run -- --id 1 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

two:
    cargo run -- --id 2 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

three:
    cargo run -- --id 3 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
