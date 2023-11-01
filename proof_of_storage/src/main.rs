#![feature(core_intrinsics)]

use std::mem::transmute_copy;
use blake3::Hasher as Blake3;
use itertools::iterate;

use fields::ft253_192::Ft253_192;
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

use ff::Field;
use merlin::Transcript;

mod fields;
mod tests;

fn main() {
    //Create a compact commit off a matrixLigero encoding with a Rho of 2
    // let test_file = File::open("proof_of_storage/test_file.txt").unwrap();
    let data: Vec<Ft253_192> = fields::read_file_to_field_elements_vec("proof_of_storage/test_file.txt");
    println!("data: {:?}\n", data);
    let data_width = (data.len() as f32).sqrt().ceil() as usize;
    let matrix_width = data_width.next_power_of_two();
    let matrix_colums = (matrix_width + 1).next_power_of_two();
    let encoding = LigeroEncoding::<Ft253_192>::new_from_dims(matrix_width, matrix_colums);
    println!("encoding: {:?}\n", encoding);
    let commit = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();
    println!("commit: {:?}\n", commit);
    let root = commit.get_root();

    let x = Ft253_192::random(&mut rand::thread_rng());

    let inner_tensor: Vec<Ft253_192> = iterate(Ft253_192::ONE, |&v| v * x)
        .take(commit.get_n_per_row())
        .collect();
    let outer_tensor: Vec<Ft253_192> = {
        let xr = x * inner_tensor.last().unwrap();
        iterate(Ft253_192::ONE, |&v| v * xr)
            .take(commit.get_n_rows())
            .collect()
    };

    let mut transcript = Transcript::new(b"test");

    let proof = commit.prove(&outer_tensor, &encoding, &mut transcript).unwrap();
    let verification = proof.verify(root.as_ref(), &outer_tensor, &inner_tensor, &encoding, &mut transcript );


    println!("verification: {:?}\n", verification);


    // let bit_width = ft253_192::Ft253_192::CAPACITY;
    // let vec_of_field_elements_from_file = fields::read_file_to_field_elements_vec::<ft253_192::Ft253_192>("proof_of_storage/test_file.txt");
    // println!("vec_of_field_elements_from_file: {:?}", vec_of_field_elements_from_file);
}


