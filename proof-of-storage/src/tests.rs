
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

#[test]
fn file_to_field_to_file(){
    use crate::fields::ft253_192::Ft253_192;
    use crate::fields::{read_file_to_field_elements_vec, field_elements_vec_to_file};
    use pretty_assertions::{assert_eq, assert_ne};

    let known_file = "test_file.txt";
    let temp_file = "temp_file.txt";

    let cleanup = Cleanup {files: vec![temp_file.to_string()]};

    let file_as_field: Vec<Ft253_192> = read_file_to_field_elements_vec(known_file);
    field_elements_vec_to_file(temp_file, &file_as_field);
    assert_eq!(std::fs::read(known_file).unwrap_or(vec![]), std::fs::read(temp_file).unwrap_or(vec![]));

    std::mem::drop(cleanup);
}