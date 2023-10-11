use ff::{Field, PrimeField};
use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding, LigeroEncodingRho};
use blake3::Hasher as Blake3;
use typenum::{U1, U2};

mod fields;
use fields::ft253_192;

fn main() {
    //Create a compact commit off a matrixLigero encoding with a Rho of 2
    let mut rng = rand::thread_rng();
    let data_len = 100;
    let rows = f64::sqrt(data_len as f64).ceil() as usize;
    let columns = rows.next_power_of_two();
    let data: Vec<ft253_192::Ft253_192> = (0..data_len).map(|_| ft253_192::Ft253_192::random(&mut rng)).collect();
    let encoding = LigeroEncoding::<ft253_192::Ft253_192>::new_from_dims(rows, columns);
    let commit = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();
    // let rho =
    print!("{}",ft253_192::Ft253_192::NUM_BITS);
}


