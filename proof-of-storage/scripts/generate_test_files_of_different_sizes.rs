use anyhow::Result;
use num_traits::pow;
use rand_core::RngCore;
use std::env;
use std::fs;
use std::io::Write;

fn main() -> Result<()> {
    let prefixes = vec![1usize, 4];
    let orders_of_magnitude: Vec<usize> = (3..=9).collect();
    let sizes: Vec<usize> = vec![];

    let mut test_files_path = env::current_dir()?;
    let intended_execution_directory = test_files_path.join("proof-of-storage");
    if intended_execution_directory.exists() {
        env::set_current_dir(&intended_execution_directory)?;
        test_files_path = intended_execution_directory;
    }

    test_files_path.push("test_files");

    let mut rng = rand::thread_rng();
    let mut buffer = [0u8; 2usize.pow(15)];

    for order_mag in orders_of_magnitude.iter() {
        for prefix in prefixes.iter() {
            let total_size = pow(10, *order_mag) * prefix;

            let file_ending = format!("{}_byte_file.bytes", total_size);
            let file_location = test_files_path.join(&file_ending);
            if file_location.exists() && file_location.metadata()?.len() == total_size as u64 {
                println!("{file_ending} already exists and has the right size");
                continue;
            }
            println!("creating the file {}", file_location.display());
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&file_location)?;

            let mut bytes_written = 0;
            let mut bytes_left = total_size;

            while bytes_left > 0 {
                let chunk_size = bytes_left.min(buffer.len());
                rng.fill_bytes(&mut buffer[0..chunk_size]);
                file.write_all(&buffer[..chunk_size])?;
                bytes_written += chunk_size;
                bytes_left -= chunk_size;
            }
        }
    }
    Ok(())
}
