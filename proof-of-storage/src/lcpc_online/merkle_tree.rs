use anyhow::{ensure, Result};
use blake3::traits::digest::{Digest, FixedOutputReset, Output};
use lcpc_2d::{log2, merkle_tree};
use std::ops::Index;
use typenum::Unsigned;

#[derive(Debug)]
pub struct MerkleTree<D: Digest + FixedOutputReset> {
    digests: Vec<Output<D>>,
    width: usize,
}

impl<D: Digest + FixedOutputReset> MerkleTree<D> {
    pub fn new(children_leaves: &[Output<D>]) -> Result<Self> {
        let width = children_leaves.len();
        ensure!(width.is_power_of_two(), "Input needs to be a power of two.");
        ensure!(width > 2, "input needs to be at least two.");
        let mut parent_leaves = vec![Output::<D>::default(); width - 1];
        merkle_tree::<D>(&children_leaves, &mut parent_leaves);

        let mut digests = Vec::with_capacity(width * 2 - 1);
        digests.extend_from_slice(&children_leaves);
        digests.extend_from_slice(&parent_leaves);

        Ok(Self { digests, width })
    }

    pub fn new_from_hashers(hashers: Vec<D>) -> Result<Self> {
        let digests: Vec<Output<D>> = hashers.into_iter().map(|h| h.finalize()).collect();
        Self::new(&digests)
    }

    pub fn root(&self) -> Output<D> {
        self.digests.last().unwrap().clone()
    }

    pub fn get_path(&self, mut index: usize) -> Option<Vec<Output<D>>> {
        if index >= self.width {
            return None;
        }
        let path_len = log2(self.width);
        let mut path = Vec::with_capacity(path_len);
        let mut digests: &[Output<D>] = &*self.digests.clone();
        for _ in 0..path_len {
            let other = (index & !1) | (!index & 1);
            assert_eq!(other ^ index, 1);
            path.push(digests[other].clone());
            let (_, hashes_new) = digests.split_at((digests.len() + 1) / 2);
            digests = hashes_new;
            index >>= 1;
        }
        assert_eq!(index, 0);
        Some(path)
    }

    pub fn len(&self) -> usize {
        self.digests.len()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.digests.iter().flat_map(|d| d.clone()).collect()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let num_digests = bytes.len() / D::OutputSize::to_usize();
        ensure!(
            (num_digests + 1).is_power_of_two(),
            "input size must be a power of two"
        );
        ensure!(
            num_digests > 2,
            "Merkle tree must be a non-trivial binary tree"
        );

        let mut digests = vec![Output::<D>::default(); num_digests];
        for (i, digest) in digests.iter_mut().enumerate() {
            let digest_bytes =
                &bytes[i * D::OutputSize::to_usize()..(i + 1) * D::OutputSize::to_usize()];
            digest.as_mut().clone_from_slice(digest_bytes);
        }
        Ok(Self {
            digests,
            width: (num_digests + 1) / 2,
        })
    }
}

impl<D: Digest + FixedOutputReset> Index<usize> for MerkleTree<D> {
    type Output = Output<D>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.digests[index]
    }
}

impl<D: Digest + FixedOutputReset> Into<Vec<Output<D>>> for MerkleTree<D> {
    fn into(self) -> Vec<Output<D>> {
        self.digests
    }
}

impl<D: Digest + FixedOutputReset> Clone for MerkleTree<D> {
    fn clone(&self) -> Self {
        Self {
            digests: self.digests.clone(),
            width: self.width,
        }
    }
}
impl<D: Digest + FixedOutputReset> PartialEq for MerkleTree<D> {
    fn eq(&self, other: &Self) -> bool {
        if self.width != other.width {
            return false;
        }
        for (digest_left, digest_right) in self.digests.iter().zip(other.digests.iter()) {
            if digest_left != digest_right {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::RandomBytesIterator;
    use blake3::Hasher as Blake3;

    #[test]
    fn to_bytes_and_back() {
        let tree_width = 1 << 5;
        let mut digests = Vec::with_capacity(tree_width);
        let mut random_byte_iter = RandomBytesIterator::new();
        for _ in 0..tree_width {
            let mut digest = Blake3::default();
            digest.update(&random_byte_iter.by_ref().take(8).collect::<Vec<u8>>());
            digests.push(digest.finalize());
        }

        let tree = MerkleTree::<Blake3>::new(&digests).unwrap();
        let bytes = tree.to_bytes();
        let new_tree = MerkleTree::<Blake3>::from_bytes(&bytes).unwrap();
        for i in 0..new_tree.digests.len() {
            assert_eq!(new_tree.digests[i], tree.digests[i]);
        }
    }
}
