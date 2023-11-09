#[cfg(test)]
mod tests {
    use ff::{Field, PrimeField};
    use lcpc_test_fields::ft63::Ft63;
    use lcpc_test_fields::random_coeffs;
    use crate::fields::ft253_192::Ft253_192;
    use crate::fields::*;
    use rand::Rng;
    use pretty_assertions::{assert_eq};
    use lcpc_2d::LcEncoding;
    use lcpc_ligero_pc::{LigeroCommit, LigeroEncoding};
    use crate::fields;

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
    fn file_to_field_to_file() {
        let known_file = "test_file.txt";
        let temp_file = "temp_file__file_to_field_to_file__test.txt";

        let cleanup = Cleanup { files: vec![temp_file.to_string()] };

        let file_as_field: Vec<TestField> = read_file_to_field_elements_vec(known_file);
        field_elements_vec_to_file(temp_file, &file_as_field);
        assert_eq!(std::fs::read(known_file).unwrap_or(vec![]), std::fs::read(temp_file).unwrap_or(vec![]));

        std::mem::drop(cleanup);
    }

    #[test]
    fn field_to_file_to_field() {
        let temp_file = "temp_file__field_to_file_to_field__test.txt";

        let cleanup = Cleanup { files: vec![temp_file.to_string()] };

        let random_field: Vec<TestField> = random_writeable_field_vec(1);
        field_elements_vec_to_file(temp_file, &random_field);
        let file_as_field: Vec<TestField> = read_file_to_field_elements_vec(temp_file);
        assert_eq!(random_field, file_as_field);
    }

    #[test]
    fn max_element_from_bytes() {
        use ff::FromUniformBytes;

        type TestField = fields::writable_ft63::WriteableFt63;
        const CAPACITY: usize = TestField::CAPACITY as usize;
        const BYTE_WIDTH: usize = CAPACITY / 8;

        let rng = &mut rand::thread_rng();
        let mut bytes_array = [0u64; 1];
        rng.fill(&mut bytes_array[..]);

        let mut max_array = [u64::MAX];

        let my_element = TestField::from_u64_array(bytes_array);
        let true_max_element = TestField::ZERO - TestField::ONE;
        let test_max_element = TestField::from_u64_array(max_array);
        println!("my_element: {:?}", my_element);
    }

    #[test]
    fn end_to_end_with_set_dimensions() {
        use blake3::Hasher as Blake3;
        use itertools::iterate;
        use merlin::Transcript;


        let data: Vec<TestField> = fields::read_file_to_field_elements_vec("test_file.txt");
        let data_min_width = (data.len() as f32).sqrt().ceil() as usize;
        let data_realized_width = data_min_width.next_power_of_two();
        let matrix_colums = (data_realized_width + 1).next_power_of_two();
        let encoding = LigeroEncoding::<TestField>::new_from_dims(data_realized_width, matrix_colums);
        let commit = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();
        let root = commit.get_root();

        // randomly select evaluation point of the polynomial
        let x = TestField::random(&mut rand::thread_rng());

        // generate the inner and outer tensor for polynomial evaluation evaluated as
        // outer_tensor.T * polynomial_matrix * inner_tensor = polynomial(x)
        // so innter_tensor = [1, x, x^2,...] and outer_tensor = [x^n, x^2n, ...]
        // where n is the polynomail matrix's number of columns
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
        let verification = proof.verify(root.as_ref(), &outer_tensor, &inner_tensor, &encoding, &mut transcript ).unwrap();
    }


    fn get_random_coeffs<T>() -> Vec<T>
        where
            T: ff::PrimeField,
    {
        use std::iter::repeat_with;

        let mut rng = rand::thread_rng();

        let lgl = 8 + rng.gen::<usize>() % 8;
        let len_base = 1 << (lgl - 1);
        let len = len_base + (rng.gen::<usize>() % len_base);

        repeat_with(|| T::random(&mut rng)).take(len).collect()
    }
    #[test]
    fn ligero_with_my_field_end_to_end() {
        use blake3::Hasher as Blake3;
        use itertools::iterate;
        use merlin::Transcript;
        type TestField = writable_ft63::WriteableFt63;
        // type TestField = Ft255;
        // type TestField = proof_of_storage::fields::ft253_192::Ft253_192;

        // commit to a random polynomial at a random rate
        let coeffs: Vec<TestField> = get_random_coeffs::<TestField>();
        let enc = LigeroEncoding::new(coeffs.len());
        let comm = LigeroCommit::<Blake3, _>::commit(&coeffs, &enc).unwrap();
        // this is the polynomial commitment
        let root = comm.get_root();

        // evaluate the random polynomial we just generated at a random point x
        let x = TestField::random(&mut rand::thread_rng());

        // compute the outer and inner tensors for powers of x
        // NOTE: we treat coeffs as a univariate polynomial, but it doesn't
        // really matter --- the only difference from a multilinear is the
        // way we compute outer_tensor and inner_tensor from the eval point
        let inner_tensor: Vec<TestField> = iterate(TestField::ONE, |&v| v * x)
            .take(comm.get_n_per_row())
            .collect();
        let outer_tensor: Vec<TestField> = {
            let xr = x * inner_tensor.last().unwrap();
            iterate(TestField::ONE, |&v| v * xr)
                .take(comm.get_n_rows())
                .collect()
        };

        // compute an evaluation proof
        let mut tr1 = Transcript::new(b"test transcript");
        tr1.append_message(b"polycommit", root.as_ref());
        tr1.append_message(b"ncols", &(enc.get_n_col_opens() as u64).to_be_bytes()[..]);
        let pf = comm.prove(&outer_tensor[..], &enc, &mut tr1).unwrap();

        // verify it and finish evaluation
        let mut tr2 = Transcript::new(b"test transcript");
        tr2.append_message(b"polycommit", root.as_ref());
        tr2.append_message(b"ncols", &(enc.get_n_col_opens() as u64).to_be_bytes()[..]);
        let enc2 = LigeroEncoding::new_from_dims(pf.get_n_per_row(), pf.get_n_cols());
        pf.verify(
            root.as_ref(),
            &outer_tensor[..],
            &inner_tensor[..],
            &enc2,
            &mut tr2,
        )
            .unwrap();
    }

    #[test]
    fn ligero_with_my_field_and_from_file_end_to_end() {
        use blake3::Hasher as Blake3;
        use itertools::iterate;
        use merlin::Transcript;
        type TestField = writable_ft63::WriteableFt63;
        // type TestField = Ft255;
        // type TestField = proof_of_storage::fields::ft253_192::Ft253_192;

        // commit to a random polynomial at a random rate
        let coeffs: Vec<TestField> = read_file_to_field_elements_vec("test_file.txt");
        let enc = LigeroEncoding::new(coeffs.len());
        let comm = LigeroCommit::<Blake3, _>::commit(&coeffs, &enc).unwrap();
        // this is the polynomial commitment
        let root = comm.get_root();

        // evaluate the random polynomial we just generated at a random point x
        let x = TestField::random(&mut rand::thread_rng());

        // compute the outer and inner tensors for powers of x
        // NOTE: we treat coeffs as a univariate polynomial, but it doesn't
        // really matter --- the only difference from a multilinear is the
        // way we compute outer_tensor and inner_tensor from the eval point
        let inner_tensor: Vec<TestField> = iterate(TestField::ONE, |&v| v * x)
            .take(comm.get_n_per_row())
            .collect();
        let outer_tensor: Vec<TestField> = {
            let xr = x * inner_tensor.last().unwrap();
            iterate(TestField::ONE, |&v| v * xr)
                .take(comm.get_n_rows())
                .collect()
        };

        // compute an evaluation proof
        let mut tr1 = Transcript::new(b"test transcript");
        tr1.append_message(b"polycommit", root.as_ref());
        tr1.append_message(b"ncols", &(enc.get_n_col_opens() as u64).to_be_bytes()[..]);
        let pf = comm.prove(&outer_tensor[..], &enc, &mut tr1).unwrap();

        // verify it and finish evaluation
        let mut tr2 = Transcript::new(b"test transcript");
        tr2.append_message(b"polycommit", root.as_ref());
        tr2.append_message(b"ncols", &(enc.get_n_col_opens() as u64).to_be_bytes()[..]);
        let enc2 = LigeroEncoding::new_from_dims(pf.get_n_per_row(), pf.get_n_cols());
        pf.verify(
            root.as_ref(),
            &outer_tensor[..],
            &inner_tensor[..],
            &enc2,
            &mut tr2,
        )
            .unwrap();
    }

    #[test]
    fn ligero_with_my_field_and_from_file_and_custom_dims_end_to_end() {
        use blake3::Hasher as Blake3;
        use itertools::iterate;
        use merlin::Transcript;
        type TestField = writable_ft63::WriteableFt63;
        // type TestField = Ft255;
        // type TestField = proof_of_storage::fields::ft253_192::Ft253_192;

        // commit to a random polynomial at a random rate
        let data: Vec<TestField> = read_file_to_field_elements_vec("test_file.txt");

        let data_min_width = (data.len() as f32).sqrt().ceil() as usize;
        let data_realized_width = data_min_width.next_power_of_two();
        let matrix_colums = (data_realized_width + 1).next_power_of_two();
        let encoding = LigeroEncoding::<TestField>::new_from_dims(data_realized_width, matrix_colums);
        let comm = LigeroCommit::<Blake3, _>::commit(&data, &encoding).unwrap();
        // this is the polynomial commitment
        let root = comm.get_root();

        // evaluate the random polynomial we just generated at a random point x
        let x = TestField::random(&mut rand::thread_rng());

        // compute the outer and inner tensors for powers of x
        // NOTE: we treat coeffs as a univariate polynomial, but it doesn't
        // really matter --- the only difference from a multilinear is the
        // way we compute outer_tensor and inner_tensor from the eval point
        let inner_tensor: Vec<TestField> = iterate(TestField::ONE, |&v| v * x)
            .take(comm.get_n_per_row())
            .collect();
        let outer_tensor: Vec<TestField> = {
            let xr = x * inner_tensor.last().unwrap();
            iterate(TestField::ONE, |&v| v * xr)
                .take(comm.get_n_rows())
                .collect()
        };

        // compute an evaluation proof
        let mut tr1 = Transcript::new(b"test transcript");
        tr1.append_message(b"polycommit", root.as_ref());
        tr1.append_message(b"ncols", &(encoding.get_n_col_opens() as u64).to_be_bytes()[..]);
        let pf = comm.prove(&outer_tensor[..], &encoding, &mut tr1).unwrap();

        // verify it and finish evaluation
        let mut tr2 = Transcript::new(b"test transcript");
        tr2.append_message(b"polycommit", root.as_ref());
        tr2.append_message(b"ncols", &(encoding.get_n_col_opens() as u64).to_be_bytes()[..]);
        let enc2 = LigeroEncoding::new_from_dims(pf.get_n_per_row(), pf.get_n_cols());
        pf.verify(
            root.as_ref(),
            &outer_tensor[..],
            &inner_tensor[..],
            &enc2,
            &mut tr2,
        )
            .unwrap();
    }
}