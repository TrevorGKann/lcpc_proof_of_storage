use std::fs::File;
use blake3::Hasher as Blake3;
use ff::{Field, PrimeField};
use rand::Rng;

use fields::ft253_192;
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};

mod fields;

fn main() {
    //Create a compact commit off a matrixLigero encoding with a Rho of 2
    let mut rng = rand::thread_rng();
    let data_len = 100;
    let rows = f64::sqrt(data_len as f64).ceil() as usize;
    let columns = rows.next_power_of_two();
    let data: Vec<ft253_192::Ft253_192> = (0..data_len).map(|_| ft253_192::Ft253_192::random(&mut rng)).collect();
    let encoding = LigeroEncoding::<ft253_192::Ft253_192>::new_from_dims(rows, columns);
    let commit = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();

    // let bit_width = ft253_192::Ft253_192::CAPACITY;
    let vec_of_field_elements_from_file = fields::read_file_to_field_elements_vec::<ft253_192::Ft253_192>("proof_of_storage/test_file.txt");
    println!("vec_of_field_elements_from_file: {:?}", vec_of_field_elements_from_file);
}


