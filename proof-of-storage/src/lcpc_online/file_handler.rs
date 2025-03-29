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
use lcpc_2d::{LcColumn, LcEncoding, LcRoot};
use lcpc_ligero_pc::LigeroEncoding;
use rayon::prelude::*;
use std::fs::{remove_dir, remove_file, rename, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::os::unix::prelude::FileExt;
use std::path::{Path, PathBuf};
use ulid::Ulid;

pub struct FileHandler<D: Digest + FixedOutputReset + Send, F: DataField, E: LcEncoding<F = F>> {
    _file_ulid: Ulid,
    pre_encoded_size: usize,
    encoded_size: usize,
    _num_rows: usize,
    total_data_bytes: usize,
    encoding: E,

    encoded_file_handle: PathBuf,
    unencoded_file_handle: PathBuf,
    merkle_tree_file_handle: PathBuf,

    unencoded_file_read_writer: File,
    encoded_file_read_writer: EncodedFileReader<F, D, E>,
    merkle_tree: MerkleTree<D>,

    _digest: PhantomData<D>,
    _field: PhantomData<F>,
}

impl<D: Digest + FixedOutputReset + Send + Sync, F: DataField>
    FileHandler<D, F, LigeroEncoding<F>>
{
    pub fn new_attach_to_existing_ulid(
        file_directory: &Path,
        ulid: &Ulid,
        pre_encoded_size: usize,
        encoded_size: usize,
    ) -> Result<Self> {
        let encoded_file_handle = file_directory.join(get_encoded_file_location_from_id(ulid));
        let unencoded_file_handle = file_directory.join(get_unencoded_file_location_from_id(ulid));
        let merkle_tree_file_handle = file_directory.join(get_merkle_file_location_from_id(ulid));
        ensure!(encoded_file_handle.is_file(), "no encoded file found!");
        ensure!(unencoded_file_handle.is_file(), "no unencoded file found!");
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

    pub fn new_attach_to_existing_files(
        ulid: &Ulid,
        unencoded_file_handle: PathBuf,
        encoded_file_handle: PathBuf,
        digest_file_handle: PathBuf,
        pre_encoded_size: usize,
        encoded_size: usize,
    ) -> Result<Self> {
        let unencoded_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&unencoded_file_handle)
            .context("couldn't open unencoded file!")?;

        let total_data_bytes = unencoded_file.metadata()?.len() as usize;
        let num_rows = total_data_bytes.div_ceil(pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
        let encoded_file_reader = EncodedFileReader::new_ligero(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&encoded_file_handle)?,
            pre_encoded_size,
            encoded_size,
        );

        let mut merkle_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&digest_file_handle)?;
        let mut merkle_bytes = Vec::new();
        merkle_file.read_to_end(&mut merkle_bytes)?;
        let merkle_tree = MerkleTree::from_bytes(&merkle_bytes)?;

        Ok(Self {
            _file_ulid: ulid.clone(),
            pre_encoded_size,
            encoded_size,
            _num_rows: num_rows,
            total_data_bytes,
            encoding: LigeroEncoding::new_from_dims(pre_encoded_size, encoded_size),
            unencoded_file_read_writer: unencoded_file,
            encoded_file_read_writer: encoded_file_reader,
            merkle_tree,
            encoded_file_handle,
            unencoded_file_handle,
            merkle_tree_file_handle: digest_file_handle,
            _digest: PhantomData,
            _field: PhantomData,
        })
    }

    pub fn create_from_unencoded_file(
        ulid: &Ulid,
        file_handle_if_not_already_ulid: Option<&PathBuf>,
        pre_encoded_size: usize,
        encoded_size: usize,
    ) -> Result<Self> {
        ensure!(
            encoded_size.is_power_of_two(),
            "encoded file size must be a power of two!"
        );

        let unencoded_path = get_unencoded_file_location_from_id(ulid);
        let encoded_path = get_encoded_file_location_from_id(ulid);
        let digest_path = get_merkle_file_location_from_id(ulid);
        if let Some(file_handle) = file_handle_if_not_already_ulid {
            rename(file_handle, &unencoded_path)?;
        }

        let mut unencoded_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&unencoded_path)?;
        let mut digest_file = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .truncate(true)
            .open(&digest_path)?;
        EncodedFileWriter::<F, D, LigeroEncoding<F>>::convert_unencoded_file(
            &mut unencoded_file,
            &encoded_path,
            Some(&mut digest_file),
            pre_encoded_size,
            encoded_size,
        )?;

        Self::new_attach_to_existing_files(
            &ulid,
            unencoded_path,
            encoded_path,
            digest_path,
            pre_encoded_size,
            encoded_size,
        )
    }

    pub fn clone_to_new_ulid(
        &self,
        _new_ulid: Ulid,
        target_directory: Option<PathBuf>,
    ) -> Result<()> {
        let _target_directory =
            target_directory.unwrap_or(self.unencoded_file_handle.parent().unwrap().to_path_buf());

        todo!()
    }

    pub fn get_encoded_file_handle(&self) -> PathBuf {
        self.encoded_file_handle.clone()
    }

    pub fn get_raw_file_handle(&self) -> PathBuf {
        self.unencoded_file_handle.clone()
    }

    pub fn get_merkle_file_handle(&self) -> PathBuf {
        self.merkle_tree_file_handle.clone()
    }

    pub fn reshape(
        &mut self,
        new_pre_encoded_columns: usize,
        new_encdoded_columns: usize,
    ) -> Result<MerkleTree<D>> {
        let mut unencoded_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&self.unencoded_file_handle)?;
        let mut merkle_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&self.merkle_tree_file_handle)?;

        EncodedFileWriter::<F, D, LigeroEncoding<F>>::convert_unencoded_file(
            &mut unencoded_file,
            &self.encoded_file_handle,
            Some(&mut merkle_file),
            new_pre_encoded_columns,
            new_encdoded_columns,
        )
    }

    // returns a the unencoded bytes that were edited, and the new edited root.
    pub fn edit_bytes(
        &mut self,
        byte_start: usize,
        unencoded_bytes_to_add: &[u8],
    ) -> Result<(Vec<u8>, MerkleTree<D>)> {
        ensure!(
            self.unencoded_file_handle.is_file(),
            "no unencoded file found!"
        );
        ensure!(
            byte_start + unencoded_bytes_to_add.len() <= self.total_data_bytes,
            "can't edit more bytes than there are in the file!"
        );

        // extract the data that was originally there and replace it with the new stuff
        // optimization: don't need to open the file every time when I have the reader

        // read and replace the original bytes
        let mut original_bytes = vec![0u8; unencoded_bytes_to_add.len()];
        if cfg!(unix) {
            self.unencoded_file_read_writer
                .read_at(&mut original_bytes, byte_start as u64)?;
            self.unencoded_file_read_writer
                .write_at(unencoded_bytes_to_add, byte_start as u64)?;
        } else {
            self.unencoded_file_read_writer
                .seek(SeekFrom::Start(byte_start as u64))?;
            self.unencoded_file_read_writer
                .read_exact(&mut original_bytes)?;

            self.unencoded_file_read_writer
                .seek(SeekFrom::Start(byte_start as u64))?;
            self.unencoded_file_read_writer
                .write_all(unencoded_bytes_to_add)?;
        }
        // self.encoded_file_read_writer
        //     .edit_decoded_bytes(byte_start, unencoded_bytes_to_add)?;
        let start_row =
            byte_start / (self.pre_encoded_size as usize * F::DATA_BYTE_CAPACITY as usize);
        let end_row = (byte_start + unencoded_bytes_to_add.len())
            .div_ceil(self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize);
        for row in start_row..end_row {
            let mut unencoded_row_buff =
                vec![0u8; self.pre_encoded_size as usize * F::DATA_BYTE_CAPACITY as usize];
            self.unencoded_file_read_writer.read_at(
                &mut unencoded_row_buff,
                (row * self.pre_encoded_size * F::DATA_BYTE_CAPACITY as usize) as _,
            )?;
            self.encoded_file_read_writer
                .replace_row_with_decoded_bytes(row, &unencoded_row_buff)?
        }

        let new_tree = self.update_merkle_file()?;
        Ok((original_bytes, new_tree))
    }

    pub fn append_bytes(&mut self, bytes_to_add: Vec<u8>) -> Result<MerkleTree<D>> {
        let mut unencoded_file_writer = OpenOptions::new()
            .append(true)
            .open(&self.unencoded_file_handle)?;

        unencoded_file_writer.write_all(&bytes_to_add)?;

        self.reencode_unencoded_file()?;
        // optimization: This needs to be changed to only affect the last row once the encoded file can grow

        self.get_merkle_tree()
    }

    pub fn get_decoded_row(&mut self, row_index: usize) -> Result<Vec<F>> {
        let encoded_row = self.get_encoded_row(row_index)?;
        let mut decoded_row = decode_row(encoded_row)?;
        decoded_row.drain(self.pre_encoded_size..);
        Ok(decoded_row)
    }

    pub fn get_decoded_row_bytes(&mut self, row_index: usize) -> Result<Vec<u8>> {
        let row = self.get_decoded_row(row_index)?;
        Ok(F::field_vec_to_byte_vec(&row))
    }

    /// only requires that a single unencoded file exists at the given handle and it will iterate over that file
    /// to produce an encoded transposed file as well as create the digest file.
    pub fn reencode_unencoded_file(&mut self) -> Result<()> {
        self.total_data_bytes = self.unencoded_file_handle.metadata()?.len() as usize;
        let mut raw_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.unencoded_file_handle)?;
        // let mut raw_file_reader = BufReader::with_capacity(F::DATA_BYTE_CAPACITY as usize * self.pre_encoded_size, raw_file);

        let new_encoded_file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.encoded_file_handle)?;

        let mut new_encoded_file_writer: EncodedFileWriter<F, D, LigeroEncoding<F>> =
            EncodedFileWriter::new(
                self.pre_encoded_size,
                self.encoded_size,
                self.total_data_bytes,
                new_encoded_file,
            );

        let mut buffer = Vec::with_capacity(F::DATA_BYTE_CAPACITY as usize * self.pre_encoded_size);

        loop {
            let bytes_read = raw_file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            new_encoded_file_writer.push_bytes(&buffer[..bytes_read])?;
        }

        let tree = new_encoded_file_writer.finalize_to_merkle_tree()?;
        self.write_tree(&tree)?;

        let new_encoded_file_reader = EncodedFileReader::new_ligero(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.encoded_file_handle)?,
            self.pre_encoded_size,
            self.encoded_size,
        );

        self.encoded_file_read_writer = new_encoded_file_reader;
        self.merkle_tree = tree;

        Ok(())
    }

    pub fn update_merkle_file(&mut self) -> Result<MerkleTree<D>> {
        let tree = self
            .encoded_file_read_writer
            .process_file_to_merkle_tree()?;
        self.merkle_tree = tree.clone();
        Ok(tree)
    }

    pub fn write_tree(&mut self, tree: &MerkleTree<D>) -> Result<()> {
        ensure!(
            tree.len() == self.encoded_size * 2 - 1,
            "this Merkle tree is the incorrect size"
        );
        let mut digest_file_writer = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.merkle_tree_file_handle)?;

        write_tree_to_file::<D>(&mut digest_file_writer, tree)?;
        Ok(())
    }

    pub fn get_encoded_row(&mut self, row_index: usize) -> Result<Vec<F>> {
        // optimization: it's probably easier to read the unencoded_row and then encode it
        self.encoded_file_read_writer.get_encoded_row(row_index)
    }

    pub fn verify_all_files_agree(&mut self) -> Result<()> {
        let recalculated_encoded_tree = self
            .encoded_file_read_writer
            .process_file_to_merkle_tree()?;

        let mut unencoded_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.unencoded_file_handle)?;
        let mut buffer = vec![0u8; F::DATA_BYTE_CAPACITY as usize * self.pre_encoded_size];
        let mut digest_accumulator: ColumnDigestAccumulator<D, F> =
            ColumnDigestAccumulator::new(self.encoded_size, ColumnsToCareAbout::All);
        loop {
            let bytes_read = unencoded_file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            let bytes_as_field = F::from_byte_vec(&buffer[..bytes_read]);
            let mut row_to_encode = vec![F::ZERO; self.encoded_size];
            for (i, field_from_bytes) in bytes_as_field.iter().enumerate() {
                row_to_encode[i] = *field_from_bytes;
            }
            self.encoding.encode(&mut row_to_encode)?;
            digest_accumulator.update(&row_to_encode)?;
        }
        let recalculated_unencoded_tree = digest_accumulator.finalize_to_merkle_tree()?;

        ensure!(recalculated_unencoded_tree == recalculated_encoded_tree);
        ensure!(recalculated_unencoded_tree == self.merkle_tree);
        Ok(())
    }
}

impl<
        D: Digest + FixedOutputReset + Send + Sync,
        F: DataField,
        E: LcEncoding<F = F> + Send + Sync,
    > FileHandler<D, F, E>
{
    pub fn read_only_digests(
        &mut self,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Result<Vec<Output<D>>> {
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

    pub fn read_full_columns(
        &mut self,
        columns_to_care_about: ColumnsToCareAbout,
    ) -> Result<Vec<LcColumn<D, E>>> {
        let column_indices = match columns_to_care_about {
            ColumnsToCareAbout::All => (0..self.encoded_size).collect(),
            ColumnsToCareAbout::Only(column_indices) => column_indices,
        };
        // let mut return_columns = Vec::with_capacity(column_indices.len());
        // for col in column_indices {
        //     return_columns.push(self.internal_open_column(col)?);
        // }
        let return_columns = column_indices
            .par_iter()
            .map(|col| -> Result<_> { self.internal_open_column(*col) })
            .collect::<Result<Vec<_>>>()?;
        Ok(return_columns)
    }

    pub fn get_unencoded_bytes(&mut self, byte_start: usize, byte_end: usize) -> Result<Vec<u8>> {
        let mut unencoded_file_reader = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.unencoded_file_handle)?;
        let mut original_byte_buffer = vec![0u8; byte_end - byte_start];
        unencoded_file_reader.read_exact(&mut original_byte_buffer)?;
        Ok(original_byte_buffer)
    }

    pub fn get_total_unencoded_bytes(&self) -> usize {
        self.total_data_bytes
    }

    pub fn get_merkle_tree(&mut self) -> Result<MerkleTree<D>> {
        Ok(self.merkle_tree.to_owned())
    }

    pub fn get_commit_root(&mut self) -> Result<LcRoot<D, E>> {
        let root = self.get_merkle_tree()?.root();
        Ok(LcRoot::<D, E>::new_from_root_digest(root))
    }

    fn internal_get_merkle_path_for_column(&self, column_index: usize) -> Result<Vec<Output<D>>> {
        ensure!(
            column_index < self.encoded_size,
            "column index out of bounds"
        );

        let Some(path) = self.merkle_tree.get_path(column_index) else {
            bail!("no path found for such an index")
        };

        Ok(path)
    }

    fn internal_get_encoded_column_without_path(&self, column_index: usize) -> Result<Vec<F>> {
        self.encoded_file_read_writer
            .get_encoded_column_without_path(column_index)
    }

    fn internal_open_column(&self, column_index: usize) -> Result<LcColumn<D, E>> {
        Ok(LcColumn::<D, E> {
            col: self.internal_get_encoded_column_without_path(column_index)?,
            path: self.internal_get_merkle_path_for_column(column_index)?,
        })
    }

    pub fn delete_all_files(self) -> Result<()> {
        remove_file(&self.unencoded_file_handle)?;
        remove_file(&self.encoded_file_handle)?;
        remove_file(&self.merkle_tree_file_handle)?;
        if self
            .unencoded_file_handle
            .parent()
            .unwrap()
            .read_dir()?
            .next()
            .is_none()
        {
            remove_dir(self.unencoded_file_handle.parent().unwrap())?
        }
        Ok(())
    }

    pub fn get_dimensions(&self) -> Result<(usize, usize, usize)> {
        Ok((self.pre_encoded_size, self.encoded_size, self._num_rows))
    }
    pub fn get_total_data_bytes(&self) -> usize {
        self.total_data_bytes
    }
}

pub fn read_tree<D: Digest + FixedOutputReset + Send>(
    tree_file: &mut File,
) -> Result<MerkleTree<D>> {
    let mut tree_bytes = vec![0u8; tree_file.metadata()?.len() as usize];
    tree_file.seek(SeekFrom::Start(0))?;
    let num_bytes_read = tree_file.read(&mut tree_bytes)?;
    assert!(
        num_bytes_read >= size_of::<Output<D>>(),
        "Merkle tree file was too small to be a valid hash"
    );
    MerkleTree::from_bytes(&tree_bytes)
}

pub fn write_tree_to_file<D: Digest + FixedOutputReset + Send>(
    tree_file: &mut File,
    tree: &MerkleTree<D>,
) -> Result<()> {
    let tree_bytes = tree.to_bytes();

    tree_file.write_all(&tree_bytes)?;
    Ok(())
}
