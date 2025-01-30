# just run tests within the PoS workspace

default:
    @just --choose

test:
    cargo nextest run -p proof-of-storage

clean: clean-db clean-bench

# clean database
clean-db:
    -rm -rf {{ source_directory() }}/proof-of-storage/PoR_Database/**
    -rm -rf {{ source_directory() }}/proof-of-storage/PoR_server_files/**

# get the size of the database on the disk
db-size:
    du -sch {{ source_directory() }}/proof-of-storage/PoR_*

proof-flame:
    cargo flamegraph --root --unit-test proof_of_storage \
      -- networking::tests::network_tests::test_file_verification_bad

build-pos-bin:
    cd {{ source_directory() }}/proof-of-storage/
    cargo build --bin pos --target-dir artifacts/ --release

bench:
    cargo bench -p proof-of-storage

bench-report:
    cd {{ source_directory() }}/target/
    zip -qr report.zip target/criterion/

clean-bench:
    -rm -rf {{ source_directory() }}/proof-of-storage/bench_files/**

#    mv report.zip ../
