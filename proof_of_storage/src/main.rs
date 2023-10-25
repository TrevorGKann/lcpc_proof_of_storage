#![feature(core_intrinsics)]

use std::fs::File;
use std::intrinsics::sqrtf32;
use blake3::Hasher as Blake3;
use ff::{Field, PrimeField};
use rand::Rng;

use fields::ft253_192;
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

mod fields;

fn main() {
    //Create a compact commit off a matrixLigero encoding with a Rho of 2
    // let test_file = File::open("proof_of_storage/test_file.txt").unwrap();
    let data: Vec<ft253_192::Ft253_192> = fields::read_file_to_field_elements_vec("proof_of_storage/test_file.txt");
    let data_width = (data.len() as f32).sqrt().ceil() as usize;
    let matrix_width = data_width.next_power_of_two();
    let matrix_colums = (matrix_width + 1).next_power_of_two();
    let encoding = LigeroEncoding::<ft253_192::Ft253_192>::new_from_dims(matrix_width, matrix_colums);
    let commit = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();
    println!("commit: {:?}", commit);

    // let bit_width = ft253_192::Ft253_192::CAPACITY;
    // let vec_of_field_elements_from_file = fields::read_file_to_field_elements_vec::<ft253_192::Ft253_192>("proof_of_storage/test_file.txt");
    // println!("vec_of_field_elements_from_file: {:?}", vec_of_field_elements_from_file);
}


