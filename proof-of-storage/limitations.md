# Limitations

- The edit function is currently done naively and, for some duration of time, the server has to keep a copy of the edited file in addition to the unedited file
  to pass the client's challenge.
    - This can be circumvented by having the server buffer the edits and keeping a copy of the updated merkle tree commitment.
    - Also, alternatively, the server could trade off more space on the disk and keep a merkle tree hash of each column instead of a linear hash.
- Edits can also be batched between verifications if the client keeps track of all their edits between verifications.
- When the number of columns gets too big the server cannot keep an entire row in memory and therefore cannot commit to certain, large files.
    - You could work around this by doing the FFT in parts but this is significantly slower
- 