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
    let mut into_values = [0, 60, 42, 59, 158, 247, 242, 112, 246, 228, 145, 198, 106, 73, 57, 183, 65, 94, 171, 55, 9, 162, 142, 33, 195, 17, 64, 232, 152, 8, 242, 249];
    into_values[0]=0;
    let my_repr = ft253_192::Ft253_192Repr(into_values);
    let my_element = ft253_192::Ft253_192::from_repr_vartime(my_repr);
    println!("{:?}",into_values);
    println!("{:?}",my_repr);
    println!("{:?}",my_element);
    println!("{:?}",ft253_192::Ft253_192::from_str_vartime("106302304611463203484338056704090686885312604975681252400313016821484155641"));
}


