/// TODO compare live signing vs original
/// simulate a live stream by iterating through all 100 fragment in /fragments
/// each iteration adds one fragment to the live stream
/// call the signing fragmented_bmff vs live_bmff
/// with all current fragments, measure the time it takes for each iterator
/// line graph comparing them
///     * live_bmff should be roughly like a sawtooth plot (window size)
///     * fragmented_bmff should be steadily increasing
// TODO add ffmpeg script to generate the fragments and add .gitignore for the fragments
use std::{path::PathBuf, process::Command, time::Instant};

use anyhow::{Context, Result, bail};
use c2pa::{Builder, Signer};
use serde::Serialize;

use crate::{cli::LiveSigning, signer::Config};

#[derive(Debug, Serialize, Default)]
struct Data {
    live: Vec<Vec<u128>>,
    og: Vec<Vec<u128>>,
}

pub struct LiveBenchmark {
    data: Data,
    dir: PathBuf,
    output: PathBuf,
    samples: usize,
    manifest: String,
}

impl LiveBenchmark {
    pub fn new(args: &LiveSigning) -> Result<Self> {
        Ok(Self {
            data: Default::default(),
            dir: args.dir.clone(),
            output: args.output.clone(),
            samples: args.samples,
            manifest: include_str!("../signer/test.json").to_string(),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        log::info!("running live...");

        if !self.dir.exists() {
            log::debug!(
                "the configured fragment directory does not exist! Creating new fragments..."
            );
            Command::new("bash")
                .arg("benchmarks/generate-fragments.sh")
                .output()?;
        }

        self.run_live()?;
        self.run_original()?;
        self.save()?;

        Ok(())
    }

    fn run_live(&mut self) -> Result<()> {
        log::info!("starting live");
        let (init, fragments) = self.get_paths()?;
        let out = self
            .dir
            .parent()
            .context("invalid output dir")?
            .join("signed_fragments")
            .join(init.file_name().context("invalid init path")?);

        let dir = out.parent().context("invalid output")?;
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }

        for num in 0..self.samples {
            log::info!("starting live run #{}/{}", num + 1, self.samples);
            let mut data = Vec::new();

            for i in 1..(fragments.len() + 1) {
                log::info!("signing {i} / {} fragment(s)", fragments.len());
                let mut builder = self.builder()?;
                let signer = self.signer()?;

                let now = Instant::now();
                builder.sign_live_bmff(&signer, &init, &fragments[0..i].to_vec(), &out, 8)?;
                data.push(now.elapsed().as_millis());
            }

            self.data.live.push(data);
            log::info!("finished live run #{}/{}", num + 1, self.samples);
        }

        log::info!("finished live");
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    fn run_original(&mut self) -> Result<()> {
        log::info!("starting original");
        let (init, fragments) = self.get_paths()?;
        let out = self
            .dir
            .parent()
            .context("invalid output dir")?
            .join("signed_fragments")
            .join(init.file_name().context("invalid init path")?);

        for num in 0..self.samples {
            let dir = out.parent().context("invalid output")?;
            if !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }

            log::info!("starting original run #{}/{}", num + 1, self.samples);
            let mut data = Vec::new();

            for i in 1..(fragments.len() + 1) {
                log::info!("signing {i} / {} fragment(s)", fragments.len());
                let mut builder = self.builder()?;
                let signer = self.signer()?;

                let now = Instant::now();
                // TODO seems like they are the same speed, maybe use the official impl just to make sure I didn't mess something up with the original?
                builder.sign_fragmented_files(&signer, &init, &fragments[0..i].to_vec(), &out)?;
                data.push(now.elapsed().as_millis());

                // remove signed file because fragmented sign only works that way
                std::fs::remove_dir_all(dir)?;
            }

            self.data.og.push(data);
            log::info!("finished original run #{}/{}", num + 1, self.samples);
        }

        log::info!("finished original");
        Ok(())
    }

    fn get_paths(&self) -> Result<(PathBuf, Vec<PathBuf>)> {
        let mut init = None;
        let mut fragments = Vec::new();

        for entry in self.dir.read_dir()? {
            let entry = entry?.path();

            if let Some(file) = entry.file_name() {
                if file.to_str().context("invalid file name")?.contains("init") {
                    match init {
                        None => {
                            init = Some(entry);
                            continue;
                        }
                        Some(_) => bail!("multiple init fragments found"),
                    }
                }
            }

            if let Some(ext) = entry.extension() {
                if ext.eq_ignore_ascii_case("m4s") {
                    fragments.push(entry);
                }
            }
        }

        fragments.sort();

        let Some(init) = init else {
            bail!("failed to find init fragment! expected one file to have <init> in its name")
        };

        Ok((init, fragments))
    }

    fn save(&self) -> Result<()> {
        let buf = serde_json::to_vec(&self.data)?;

        std::fs::write(&self.output, &buf)?;

        Ok(())
    }

    fn builder(&self) -> Result<Builder> {
        Ok(Builder::from_json(&self.manifest)?)
    }

    fn signer(&self) -> Result<Box<dyn Signer>> {
        Config::from_json(&self.manifest)
    }
}
