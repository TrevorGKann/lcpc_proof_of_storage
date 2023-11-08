use ff::{Field, PrimeField};
use lcpc_test_fields::ft63::Ft63;
use lcpc_test_fields::random_coeffs;
use crate::fields::ft253_192::Ft253_192;
use crate::fields::*;
use pretty_assertions::{assert_eq};
use rand::Rng;

const CLEANUP_VALUES: bool = true;
struct Cleanup {
    files: Vec<String>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        //delete all files in `files` if they exist
        if CLEANUP_VALUES {
            for file in &self.files {
                if std::path::Path::new(file).exists() {
                    std::fs::remove_file(file).unwrap();
                }
            }
        }
    }
}


type TestField = writable_ft63::WriteableFt63;

#[test]
fn file_to_field_to_file(){

    let known_file = "test_file.txt";
    let temp_file = "temp_file__file_to_field_to_file__test.txt";

    let cleanup = Cleanup {files: vec![temp_file.to_string()]};

    let file_as_field: Vec<TestField> = read_file_to_field_elements_vec(known_file);
    field_elements_vec_to_file(temp_file, &file_as_field);
    assert_eq!(std::fs::read(known_file).unwrap_or(vec![]), std::fs::read(temp_file).unwrap_or(vec![]));

    std::mem::drop(cleanup);
}

#[test]
fn field_to_file_to_field(){
    let temp_file = "temp_file__field_to_file_to_field__test.txt";

    let cleanup = Cleanup {files: vec![temp_file.to_string()]};

    let random_field: Vec<TestField> = random_writeable_field_vec(1);
    field_elements_vec_to_file(temp_file, &random_field);
    let file_as_field: Vec<TestField> = read_file_to_field_elements_vec(temp_file);
    assert_eq!(random_field, file_as_field);
}

#[test]
fn trying_out_from_uniform_bytes (){
    use ff::FromUniformBytes;

    type TestField = Ft63;
    const CAPACITY: usize = TestField::CAPACITY as usize;
    const BYTE_WIDTH: usize = CAPACITY / 8;

    let rng = &mut rand::thread_rng();
    let mut bytes_array = [0u64; 1];
    rng.fill(&mut bytes_array[..]);

    let mut max_array = [u64::MAX];

    let my_element = Ft63::from_array(bytes_array);
    let true_max_element = Ft63::ZERO-Ft63::ONE;
    let test_max_element = Ft63::from_array(max_array);
    println!("my_element: {:?}", my_element);

}