# just run tests within the PoS workspace

default:
    @just --choose

test:
    cargo nextest run -p proof-of-storage

# clean database
cleanDB:
    -rm -r {{ source_directory() }}/proof-of-storage/PoR_Database/**
    -rm -r {{ source_directory() }}/proof-of-storage/PoR_server_files/**

# get the size of the database on the disk
db-size:
    du -sch {{ source_directory() }}/proof-of-storage/PoR_*

proof-bench:
    cargo flamegraph --root --unit-test proof_of_storage -- networking::tests::network_tests::test_file_verification_bad
