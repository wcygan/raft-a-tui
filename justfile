copy:
    repomix --stdout | pbcopy

build:
    cargo build

check:
    cargo check

test:
    cargo test

one:
    cargo run -p raft-server -- --id 1 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

two:
    cargo run -p raft-server -- --id 2 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

three:
    cargo run -p raft-server -- --id 3 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

client:
    cargo run -p raft-client -- --peers 127.0.0.1:7001,127.0.0.1:7002,127.0.0.1:7003
