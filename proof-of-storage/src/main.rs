#![feature(core_intrinsics)]

use std::mem::transmute_copy;
use blake3::Hasher as Blake3;
use itertools::iterate;
use ff::{Field, PrimeField};
use fffft::FieldFFT;
use merlin::Transcript;

use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};
use lcpc_2d::LcEncoding;
use proof_of_storage::fields;
use lcpc_test_fields;

// type TestField = fields::ft253_192::Ft253_192;
type TestField = lcpc_test_fields::ft63::Ft63;

fn main() {
    //Create a compact commit off a matrixLigero encoding with a Rho of 2
    // let test_file = File::open("proof-of-storage/test_file.txt").unwrap();

    println!("TestField::S = {:?}", <TestField as FieldFFT>::S);

    let data: Vec<TestField> = fields::read_file_to_field_elements_vec("proof-of-storage/test_file.txt");
    let data_width = (data.len() as f32).sqrt().ceil() as usize;
    let matrix_width = data_width.next_power_of_two();
    let matrix_colums = (matrix_width + 1).next_power_of_two();
    let encoding = LigeroEncoding::<TestField>::new_from_dims(matrix_width, matrix_colums);
    let commit = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();
    let root = commit.get_root();

    let x = TestField::random(&mut rand::thread_rng());

    let inner_tensor: Vec<TestField> = iterate(TestField::ONE, |&v| v * x)
        .take(commit.get_n_per_row())
        .collect();
    let outer_tensor: Vec<TestField> = {
        let xr = x * inner_tensor.last().unwrap();
        iterate(TestField::ONE, |&v| v * xr)
            .take(commit.get_n_rows())
            .collect()
    };

    let mut transcript = Transcript::new(b"test");
    transcript.append_message(b"polycommit", root.as_ref());
    transcript.append_message(b"ncols", &(encoding.get_n_col_opens() as u64).to_be_bytes()[..]);

    let proof = commit.prove(&outer_tensor, &encoding, &mut transcript).unwrap();

    let second_encoding =  LigeroEncoding::<TestField>::new_from_dims(matrix_width, matrix_colums);
    let verification = proof.verify(root.as_ref(), &outer_tensor, &inner_tensor, &encoding, &mut transcript );


    println!("verification: {:?}\n", verification);


    // let bit_width = ft253_192::Ft253_192::CAPACITY;
    // let vec_of_field_elements_from_file = fields::read_file_to_field_elements_vec::<ft253_192::Ft253_192>("proof-of-storage/test_file.txt");
    // println!("vec_of_field_elements_from_file: {:?}", vec_of_field_elements_from_file);
}


