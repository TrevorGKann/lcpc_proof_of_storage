use crate::fields::data_field::DataField;
use crate::lcpc_online::column_digest_accumulator::{ColumnDigestAccumulator, ColumnsToCareAbout};
use crate::lcpc_online::decode_row;
use crate::lcpc_online::encoded_file_reader::EncodedFileReader;
use crate::lcpc_online::encoded_file_writer::EncodedFileWriter;
use crate::lcpc_online::file_formatter::{
    get_encoded_file_location_from_id, get_merkle_file_location_from_id,
    get_unencoded_file_location_from_id,
};
use crate::lcpc_online::merkle_tree::MerkleTree;
use anyhow::{bail, ensure, Context, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{LcColumn, LcEncoding};
use lcpc_ligero_pc::{LigeroEncoding, LigeroEncodingRho};
use std::fs::rename;
use std::io::SeekFrom;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufWriter};
use ulid::Ulid;

pub enum CurrentState<D: Digest + FixedOutputReset, F: DataField, E: LcEncoding<F = F>> {
    StreamingToFileCreation {
        encoded_file_writer: EncodedFileWriter<F, D, E>,
        unencoded_file_writer: BufWriter<File>,
    },
    FilesAlreadyCreated {
        encoded_file_read_writer: EncodedFileReader<F, D, E>,
        merkle_tree: MerkleTree<D>,
    },
    OnlyUnencodedFilesCreated {
        unencoded_file_writer: BufWriter<File>,
    },
}

pub struct FileHandler<D: Digest + FixedOutputReset, F: DataField, E: LcEncoding<F = F>> {
    file_ulid: Ulid,
    pre_encoded_size: usize,
    encoded_size: usize,
    num_rows: usize,
    total_data_bytes: usize,
    encoding: E,

    current_state: CurrentState<D, F, E>,

    encoded_file_handle: PathBuf,
    unencoded_file_handle: PathBuf,
    merkle_tree_file_handle: PathBuf,

    _digest: PhantomData<D>,
    _field: PhantomData<F>,
}

impl<D: Digest + FixedOutputReset, F: DataField> FileHandler<D, F, LigeroEncoding<F>> {
    // pub fn new_create_new_encoded_files(
    //     ulid: Ulid,
    //     pre_encoded_size: usize,
    //     encoded_size: usize,
    //     total_data_bytes: usize,
    // ) -> Result<Self> {
    //     todo!()
    // }

    pub async fn new_attach_to_existing_ulid(
        file_directory: &impl AsRef<Path>,
        ulid: Ulid,
        pre_encoded_size: usize,
        encoded_size: usize,
    ) -> Result<Self> {

        let encoded_file_handle = file_directory.clone().push(get_encoded_file_location_from_id(&ulid));
        ensure!(encoded_file_handle.is_file(), "no encoded file found!");
        let unencoded_file_handle = file_directory.clone().push(get_unencoded_file_location_from_id(&ulid));
        ensure!(unencoded_file_handle.is_file(), "no unencoded file found!");
        let merkle_tree_file_handle = file_directory.clone().push(get_merkle_file_location_from_id(&ulid));
        ensure!(merkle_tree_file_handle.is_file(), "no merkle file found!");

        Self::new_attach_to_existing_files(
            ulid,
            unencoded_file_handle,
            encoded_file_handle,
            merkle_tree_file_handle,
            pre_encoded_size,
            encoded_size,
        )
    }

    pub async fn new_attach_to_existing_files(
        ulid: Ulid,
        unencoded_file_handle: PathBuf,
        encoded_file_handle: PathBuf,
        digest_file_handle: PathBuf,
        pre_encoded_size: usize,
        encoded_size: usize,
    ) -> Result<Self> {
        let unencoded_file = File::open(unencoded_file_handle).await
            .context("couldn't open unencoded file!")?;

        let total_data_bytes = unencoded_file.metadata().await?.len() as usize;
        let num_rows = total_data_bytes.div_ceil(pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
        let encoded_file_reader = EncodedFileReader::new_ligero(
            File::open(encoded_file_handle).await?,
            pre_encoded_size,
            encoded_size,
        );

        let mut merkle_file = File::open(digest_file_handle).await?;
        let mut merkle_bytes = Vec::new();
        merkle_file.read_to_end(&mut merkle_bytes).await?;
        let merkle_tree = MerkleTree::from_bytes(&merkle_bytes)?;


        Ok(Self {
            file_ulid: ulid,
            pre_encoded_size,
            encoded_size,
            num_rows,
            total_data_bytes,
            encoding: LigeroEncoding::new_from_dims(pre_encoded_size, encoded_size),
            current_state: CurrentState::FilesAlreadyCreated {
                encoded_file_read_writer: encoded_file_reader,
                merkle_tree,
            },
            encoded_file_handle,
            unencoded_file_handle,
            merkle_tree_file_handle: digest_file_handle,
            _digest: PhantomData,
            _field: PhantomData,
        })
    }

    pub async fn create_from_unencoded_file(
        ulid: Ulid,
        file_handle_thats_not_already_ulid: Option<&PathBuf>,
        pre_encoded_size: usize,
        encoded_size: usize,
    ) -> Result<Self> {
        let unencoded_path = get_unencoded_file_location_from_id(&ulid);
        let encoded_path = get_encoded_file_location_from_id(&ulid);
        let digest_path = get_merkle_file_location_from_id(&ulid);
        if let Some(file_handle) = file_handle_thats_not_already_ulid {
            rename(file_handle, &unencoded_path)?;
        }

        let mut unencoded_file = File::open(unencoded_path).await?;
        let mut digest_file = OpenOptions::default()
            .create(true)
            .write(true)
            .open(digest_path)
            .await?;
        EncodedFileWriter::<F, D, LigeroEncoding<F>>::convert_unencoded_file(
            &mut unencoded_file,
            &encoded_path,
            Some(&mut digest_file),
            pre_encoded_size,
            encoded_size,
        )
        .await?;

        Self::new_attach_to_existing_files(
            ulid,
            unencoded_path,
            encoded_path,
            digest_path,
            pre_encoded_size,
            encoded_size,
        )
    }

    pub fn clone_to_new_ulid(&self, new_ulid: Ulid, target_directory: Option<PathBuf>) -> Result<()> {
        let target_directory = target_directory.unwrap_or(self.unencoded_file_handle.parent().unwrap().to_path_buf());

        todo!()
    }

    // returns a the unencoded bytes that were edited, and the new edited root.
    pub async fn edit_bytes(
        &mut self,
        byte_start: usize,
        unencoded_bytes_to_add: Vec<u8>,
    ) -> Result<(Vec<u8>, MerkleTree<D>)> {
        ensure!(
            self.unencoded_file_handle.is_file(),
            "no unencoded file found!"
        );
        ensure!(
            !matches!(
                self.current_state,
                CurrentState::StreamingToFileCreation { .. }
            ),
            "Can't edit a file that's being streamed!"
        );

        // extract the data that was originally there and replace it with the new stuff
        let mut original_file = OpenOptions::new()
            .write(true)
            .open(&self.unencoded_file_handle)
            .await?;

        let mut original_bytes = vec![0u8; unencoded_bytes_to_add.len()];
        original_file
            .seek(SeekFrom::Start(byte_start as u64))
            .await?;
        original_file.read_exact(&mut original_bytes).await?;

        original_file
            .seek(SeekFrom::Start(byte_start as u64))
            .await?;
        original_file
            .write_all(unencoded_bytes_to_add.as_slice())
            .await?;

        match &mut self.current_state {
            CurrentState::StreamingToFileCreation {
                ref encoded_file_writer,
                ref unencoded_file_writer,
            } => {
                unreachable!()
            }
            CurrentState::FilesAlreadyCreated {
                encoded_file_read_writer: ref mut read_writer,
                ..
            } => {
                // now edit piecewise the rows in place, this is cheaper than recreating the entire encoded file
                read_writer
                    .edit_row(byte_start, unencoded_bytes_to_add)
                    .await?;
                let tree = self.recalculate_merkle_file().await?;
                return Ok((original_bytes, tree));
            }
            CurrentState::OnlyUnencodedFilesCreated { .. } => {
                self.reencode_unencoded_file().await?;
                let CurrentState::FilesAlreadyCreated { merkle_tree, .. } = &self.current_state
                else {
                    bail!("something went wrong in reencoding the unencoded file")
                };
                return Ok((original_bytes, merkle_tree.clone()));
            }
        }
    }

    pub async fn append_bytes(&mut self, bytes_to_add: Vec<u8>) -> Result<MerkleTree<D>> {
        let mut unencoded_file_writer = OpenOptions::new()
            .append(true)
            .open(&self.unencoded_file_handle)
            .await?;

        unencoded_file_writer.write_all(&bytes_to_add).await?;

        self.reencode_unencoded_file().await?;

        self.get_merkle_tree()
    }

    pub async fn get_decoded_row(&mut self, row_index: usize) -> Result<Vec<F>> {
        let encoded_row = self.get_encoded_row(row_index).await?;
        let mut decoded_row = decode_row(encoded_row)?;
        decoded_row.drain(self.pre_encoded_size..);
        Ok(decoded_row)
    }

    pub async fn get_decoded_row_bytes(&mut self, row_index: usize) -> Result<Vec<u8>> {
        let row = self.get_decoded_row(row_index).await?;
        Ok(F::field_vec_to_byte_vec(&row))
    }

    /// only requires that a single unencoded file exists at the given handle and it will iterate over that file
    /// to produce an encoded transposed file as well as create the digest file.
    pub async fn reencode_unencoded_file(&mut self) -> Result<()> {
        self.total_data_bytes = self.unencoded_file_handle.metadata()?.len() as usize;
        let mut raw_file = File::open(&self.unencoded_file_handle).await?;
        // let mut raw_file_reader = BufReader::with_capacity(F::DATA_BYTE_CAPACITY as usize * self.pre_encoded_size, raw_file);

        let new_encoded_file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.encoded_file_handle)
            .await?;

        let mut new_encoded_file_writer: EncodedFileWriter<F, D, LigeroEncoding<F>> =
            EncodedFileWriter::new(
                self.pre_encoded_size,
                self.encoded_size,
                self.total_data_bytes,
                new_encoded_file,
            );

        let mut buffer = Vec::with_capacity(F::DATA_BYTE_CAPACITY as usize * self.pre_encoded_size);

        loop {
            let bytes_read = raw_file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }

            new_encoded_file_writer
                .push_bytes(&buffer[..bytes_read])
                .await?;
        }

        let tree = new_encoded_file_writer.finalize_to_merkle_tree().await?;
        self.write_tree(&tree).await?;

        let new_encoded_file_reader = EncodedFileReader::new_ligero(
            File::open(&self.encoded_file_handle).await?,
            self.pre_encoded_size,
            self.encoded_size,
        )
        .await;

        self.current_state = CurrentState::FilesAlreadyCreated {
            encoded_file_read_writer: new_encoded_file_reader,
            merkle_tree: tree,
        };

        Ok(())
    }

    pub async fn recalculate_merkle_file(&mut self) -> Result<MerkleTree<D>> {
        let mut opened_encoded_file = File::open(&self.encoded_file_handle).await?;
        let mut encoded_file_reader: EncodedFileReader<F, D, LigeroEncoding<F>> =
            EncodedFileReader::new_ligero(
                opened_encoded_file,
                self.pre_encoded_size,
                self.encoded_size,
            )
            .await;
        let tree = encoded_file_reader.process_file_to_merkle_tree().await?;
        Ok(tree)
    }

    pub async fn write_tree(&mut self, tree: &MerkleTree<D>) -> Result<()> {
        ensure!(
            tree.len() == self.encoded_size * 2 - 1,
            "this Merkle tree is the incorrect size"
        );
        let mut digest_file_writer = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.merkle_tree_file_handle)
            .await?;

        write_tree_to_file::<D>(&mut digest_file_writer, tree).await?;
        Ok(())
    }

    pub async fn get_encoded_row(&mut self, row_index: usize) -> Result<Vec<F>> {
        if !matches!(self.current_state, CurrentState::FilesAlreadyCreated { .. }) {
            self.reencode_unencoded_file().await?;
        }
        let CurrentState::FilesAlreadyCreated {
            ref mut encoded_file_read_writer,
            merkle_tree,
        } = &mut self.current_state
        else {
            bail!("error recalculating files from unencoded file")
        };
        encoded_file_read_writer.get_encoded_row(row_index).await
    }

    pub async fn verify_all_files_agree(&mut self) -> Result<()> {
        ensure!(
            matches!(self.current_state, CurrentState::FilesAlreadyCreated { .. }),
            "can't verify alternative files when none exist or are known about"
        );
        let CurrentState::FilesAlreadyCreated { merkle_tree, .. } = &self.current_state else {
            unreachable!()
        };

        let mut encoded_file = File::open(&self.encoded_file_handle).await?;
        let mut encoded_file_reader: EncodedFileReader<F, D, LigeroEncoding<F>> =
            EncodedFileReader::new_ligero(encoded_file, self.pre_encoded_size, self.encoded_size)
                .await;
        let recalculated_encoded_tree = encoded_file_reader.process_file_to_merkle_tree().await?;

        let mut unencoded_file = File::open(&self.unencoded_file_handle).await?;
        let mut buffer = vec![0u8; F::DATA_BYTE_CAPACITY as usize * self.pre_encoded_size];
        let mut digest_accumulator: ColumnDigestAccumulator<D, F> =
            ColumnDigestAccumulator::new(self.encoded_size, ColumnsToCareAbout::All);
        loop {
            let bytes_read = unencoded_file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            let mut bytes_as_field = F::from_byte_vec(&buffer[..bytes_read]);
            let mut row_to_encode = vec![F::ZERO; self.encoded_size];
            for (i, field_from_bytes) in bytes_as_field.iter().enumerate() {
                row_to_encode[i] = *field_from_bytes;
            }
            self.encoding.encode(&mut row_to_encode)?;
            digest_accumulator.update(&row_to_encode)?;
        }
        let recalculated_unencoded_tree = digest_accumulator.finalize_to_merkle_tree()?;

        ensure!(recalculated_unencoded_tree == recalculated_encoded_tree);
        ensure!(recalculated_unencoded_tree == *merkle_tree);
        Ok(())
    }
}

impl<D: Digest + FixedOutputReset, F: DataField, E: LcEncoding<F = F>> FileHandler<D, F, E> {
    pub async fn read_only_digests(
        &mut self,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Result<Vec<Output<D>>> {
        ensure!(
            matches!(self.current_state, CurrentState::FilesAlreadyCreated { .. }),
            "uninitialized file"
        );
        let column_indices = match columns_to_care_about {
            ColumnsToCareAbout::All => (0..self.encoded_size).collect(),
            ColumnsToCareAbout::Only(column_indices) => column_indices,
        };
        let tree = self.get_merkle_tree()?;
        let mut return_digests = Vec::with_capacity(column_indices.len());
        for col in column_indices {
            return_digests.push(tree[col].clone());
        }
        Ok(return_digests)
    }

    pub async fn reshape(
        &mut self,
        new_pre_encoded_columns: usize,
        new_encdoded_columns: usize,
    ) -> Result<MerkleTree<D>> {
        ensure!(!matches!(self.current_state, CurrentState::StreamingToFileCreation { .. }), "uninitialized files");

        let mut unencoded_file = File::open(&self.unencoded_file_handle).await?;
        let mut merkle_file = File::open(&self.merkle_tree_file_handle).await?;

        EncodedFileWriter::convert_unencoded_file(
            &mut unencoded_file,
            &self.encoded_file_handle,
            Some(&mut merkle_file),
            new_pre_encoded_columns,
            new_encdoded_columns
        ).await
    }

    pub async fn read_full_columns(
        &mut self,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Result<Vec<LcColumn<D, E>>> {
        let column_indices = match columns_to_care_about {
            ColumnsToCareAbout::All => (0..self.encoded_size).collect(),
            ColumnsToCareAbout::Only(column_indices) => column_indices,
        };
        let mut return_columns = Vec::with_capacity(column_indices.len());
        for col in column_indices {
            return_columns.push(self.internal_open_column(col).await?);
        }
        Ok(return_columns)
    }

    pub async fn get_unencoded_bytes(
        &mut self,
        byte_start: usize,
        byte_end: usize,
    ) -> Result<Vec<u8>> {
        let mut unencoded_file_reader = File::open(&self.unencoded_file_handle).await?;
        let mut original_byte_buffer = vec![0u8; byte_end - byte_start];
        unencoded_file_reader
            .read_exact(&mut original_byte_buffer)
            .await?;
        Ok(original_byte_buffer)

        // // I forgot I just have the original file available. Keeping this just in case I need it for
        // // some obscure case where I only have the encoded file
        // let start_byte_in_row =
        //     byte_start % (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
        // let start_row = byte_start / (self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
        // let end_row = byte_end / (self.encoded_size * F::DATA_BYTE_CAPACITY as usize);
        // let total_bytes = byte_end - byte_start;
        // let mut gathered_bytes = Vec::with_capacity(total_bytes);
        //
        // for row in start_row..=end_row {
        //     let decoded_row_bytes = self.get_decoded_row_bytes(row).await?;
        //     gathered_bytes.extend_from_slice(&decoded_row_bytes);
        // }
        //
        // gathered_bytes.drain(..start_byte_in_row);
        // gathered_bytes.drain(total_bytes..);
        // Ok(gathered_bytes)
    }

    pub fn get_total_unencoded_bytes(&self) -> usize {
        self.total_data_bytes
    }

    pub fn get_merkle_tree(&mut self) -> Result<MerkleTree<D>> {
        let CurrentState::FilesAlreadyCreated {
            merkle_tree: tree, ..
        } = &mut self.current_state
        else {
            bail!("must be in a finished state")
        };

        Ok(tree.to_owned())
    }

    async fn internal_get_merkle_path_for_column(
        &mut self,
        column_index: usize,
    ) -> Result<Vec<Output<D>>> {
        ensure!(
            column_index < self.encoded_size,
            "column index out of bounds"
        );
        let CurrentState::FilesAlreadyCreated {
            merkle_tree: ref tree,
            ..
        } = &mut self.current_state
        else {
            bail!("must be in a finished state")
        };

        let Some(path) = tree.get_path(column_index) else {
            bail!("no path found for such an index")
        };

        Ok(path)
    }

    async fn internal_get_encoded_column_without_path(
        &mut self,
        column_index: usize,
    ) -> Result<Vec<F>> {
        let CurrentState::FilesAlreadyCreated {
            encoded_file_read_writer: ref mut reader,
            ..
        } = &mut self.current_state
        else {
            bail!("must be in a finished state")
        };
        reader.get_encoded_column_without_path(column_index).await
    }

    async fn internal_open_column(&mut self, column_index: usize) -> Result<LcColumn<D, E>> {
        Ok(LcColumn::<D, E> {
            col: self
                .internal_get_encoded_column_without_path(column_index)
                .await?,
            path: self
                .internal_get_merkle_path_for_column(column_index)
                .await?,
        })
    }
}

pub async fn read_tree<D: Digest + FixedOutputReset>(
    tree_file: &mut File,
) -> Result<MerkleTree<D>> {
    let mut tree_bytes = vec![0u8; tree_file.metadata().await?.len() as usize];
    tree_file.seek(SeekFrom::Start(0)).await?;
    tree_file.read(&mut tree_bytes).await?;
    MerkleTree::from_bytes(&tree_bytes)
}

pub async fn write_tree_to_file<D: Digest + FixedOutputReset>(
    tree_file: &mut File,
    tree: &MerkleTree<D>,
) -> Result<()> {
    let tree_bytes = tree.to_bytes();

    tree_file.write_all(&tree_bytes).await?;
    Ok(())
}
