# just run tests within the PoS workspace
test:
    cargo nextest run -p proof-of-storage

# clean database
cleanDB:
    -rm -r proof-of-storage/PoR_Database/**
    -rm -r proof-of-storage/PoR_server_files/**
