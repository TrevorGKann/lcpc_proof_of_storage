#![allow(unused)]
use criterion::profiler::Profiler;
use pprof::flamegraph::color::MultiPalette;
use pprof::flamegraph::Palette;
use pprof::protos::Message;
use pprof::{flamegraph::Options, ProfilerGuard};
use std::fs::File;
use std::os::raw::c_int;
use std::path::Path;

pub struct FlamegraphProfiler<'a> {
    frequency: c_int,
    active_profiler: Option<ProfilerGuard<'a>>,
}

impl<'a> FlamegraphProfiler<'a> {
    pub fn new(frequency: c_int) -> Self {
        Self {
            frequency,
            active_profiler: None,
        }
    }
}

impl<'a> Profiler for FlamegraphProfiler<'a> {
    fn start_profiling(&mut self, _benchmark_id: &str, _benchmark_dir: &Path) {
        self.active_profiler = Some(ProfilerGuard::new(self.frequency).unwrap())
    }

    fn stop_profiling(&mut self, _benchmark_id: &str, benchmark_dir: &Path) {
        std::fs::create_dir_all(benchmark_dir).unwrap();
        let flamegraph_path = benchmark_dir.join("flamegraph.svg");
        let flamegraph_file = File::create(&flamegraph_path)
            .expect("File system error while creating flamegraph.svg");
        let data_path = benchmark_dir.join("profile.pb");
        let mut data_file =
            File::create(data_path).expect("File system error while creating data.pprof");
        let mut flamegraph_options = Options::default();
        flamegraph_options.colors = Palette::Multi(MultiPalette::Rust);

        if let Some(profiler) = self.active_profiler.take() {
            let report = profiler.report().build().unwrap();
            report
                .flamegraph_with_options(flamegraph_file, &mut flamegraph_options)
                .expect("Error writing flamegraph");
            report
                .pprof()
                .unwrap()
                .write_to_writer(&mut data_file)
                .unwrap();
        }
    }
}
