# just run tests within the PoS workspace

default:
    @just --choose

test:
    cargo nextest run -p proof-of-storage

# clean database
cleanDB:
    -rm -r {{ source_directory() }}/proof-of-storage/PoR_Database/**
    -rm -r {{ source_directory() }}/proof-of-storage/PoR_server_files/**

#{{source_directory()}}
