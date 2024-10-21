
# just run tests within the PoS workspace
test:
  cargo test -p proof-of-storage

# clean database
cleanDB:
  rm -r proof-of-storage/PoR_Database/**
  rm -r proof-of-storage/PoR_server_files/**