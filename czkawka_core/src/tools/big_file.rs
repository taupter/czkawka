use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Sender;
use fun_time::fun_time;
use humansize::{BINARY, format_size};
use log::debug;
use rayon::prelude::*;

use crate::common::WorkContinueStatus;
use crate::common_dir_traversal::{DirTraversalBuilder, DirTraversalResult, FileEntry, ToolType};
use crate::common_tool::{CommonData, CommonToolData, DeleteItemType, DeleteMethod};
use crate::common_traits::{DebugPrint, DeletingItems, PrintResults};
use crate::progress_data::ProgressData;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum SearchMode {
    BiggestFiles,
    SmallestFiles,
}

#[derive(Debug, Default)]
pub struct Info {
    pub number_of_real_files: usize,
}

pub struct BigFileParameters {
    pub number_of_files_to_check: usize,
    pub search_mode: SearchMode,
}

impl BigFileParameters {
    pub fn new(number_of_files: usize, search_mode: SearchMode) -> Self {
        Self {
            number_of_files_to_check: number_of_files.max(1),
            search_mode,
        }
    }
}

pub struct BigFile {
    common_data: CommonToolData,
    information: Info,
    big_files: Vec<FileEntry>,
    params: BigFileParameters,
}

impl BigFile {
    pub fn new(params: BigFileParameters) -> Self {
        Self {
            common_data: CommonToolData::new(ToolType::BigFile),
            information: Info::default(),
            big_files: Default::default(),
            params,
        }
    }

    #[fun_time(message = "find_big_files", level = "info")]
    pub fn find_big_files(&mut self, stop_flag: &Arc<AtomicBool>, progress_sender: Option<&Sender<ProgressData>>) {
        self.prepare_items();
        if self.look_for_big_files(stop_flag, progress_sender) == WorkContinueStatus::Stop {
            self.common_data.stopped_search = true;
            return;
        }
        if self.delete_files(stop_flag, progress_sender) == WorkContinueStatus::Stop {
            self.common_data.stopped_search = true;
            return;
        };
        self.debug_print();
    }

    #[fun_time(message = "look_for_big_files", level = "debug")]
    fn look_for_big_files(&mut self, stop_flag: &Arc<AtomicBool>, progress_sender: Option<&Sender<ProgressData>>) -> WorkContinueStatus {
        let result = DirTraversalBuilder::new()
            .group_by(|_fe| ())
            .stop_flag(stop_flag)
            .progress_sender(progress_sender)
            .common_data(&self.common_data)
            .minimal_file_size(1)
            .maximal_file_size(u64::MAX)
            .build()
            .run();

        match result {
            DirTraversalResult::SuccessFiles { grouped_file_entries, warnings } => {
                let mut all_files = grouped_file_entries.into_values().flatten().collect::<Vec<_>>();
                all_files.par_sort_unstable_by_key(|fe| fe.size);

                if self.get_params().search_mode == SearchMode::BiggestFiles {
                    all_files.reverse();
                }

                if all_files.len() > self.get_params().number_of_files_to_check {
                    all_files.truncate(self.get_params().number_of_files_to_check);
                }

                self.big_files = all_files;

                self.common_data.text_messages.warnings.extend(warnings);
                self.information.number_of_real_files = self.big_files.len();
                debug!("check_files - Found {} biggest/smallest files.", self.big_files.len());
                WorkContinueStatus::Continue
            }

            DirTraversalResult::Stopped => WorkContinueStatus::Stop,
        }
    }
}

impl DeletingItems for BigFile {
    #[fun_time(message = "delete_files", level = "debug")]
    fn delete_files(&mut self, stop_flag: &Arc<AtomicBool>, progress_sender: Option<&Sender<ProgressData>>) -> WorkContinueStatus {
        match self.common_data.delete_method {
            DeleteMethod::Delete => self.delete_simple_elements_and_add_to_messages(stop_flag, progress_sender, DeleteItemType::DeletingFiles(self.big_files.clone())),
            DeleteMethod::None => WorkContinueStatus::Continue,
            _ => unreachable!(),
        }
    }
}

impl DebugPrint for BigFile {
    #[allow(clippy::print_stdout)]
    fn debug_print(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        println!("### INDIVIDUAL DEBUG PRINT ###");
        println!("Info: {:?}", self.information);
        println!("Number of files to check - {}", self.get_params().number_of_files_to_check);
        self.debug_print_common();
        println!("-----------------------------------------");
    }
}

impl PrintResults for BigFile {
    fn write_results<T: Write>(&self, writer: &mut T) -> std::io::Result<()> {
        writeln!(
            writer,
            "Results of searching {:?} with excluded directories {:?} and excluded items {:?}",
            self.common_data.directories.included_directories,
            self.common_data.directories.excluded_directories,
            self.common_data.excluded_items.get_excluded_items()
        )?;

        if self.information.number_of_real_files != 0 {
            if self.get_params().search_mode == SearchMode::BiggestFiles {
                writeln!(writer, "{} the biggest files.\n\n", self.information.number_of_real_files)?;
            } else {
                writeln!(writer, "{} the smallest files.\n\n", self.information.number_of_real_files)?;
            }
            for file_entry in &self.big_files {
                writeln!(
                    writer,
                    "{} ({}) - \"{}\"",
                    format_size(file_entry.size, BINARY),
                    file_entry.size,
                    file_entry.path.to_string_lossy()
                )?;
            }
        } else {
            writeln!(writer, "Not found any files.")?;
        }

        Ok(())
    }

    fn save_results_to_file_as_json(&self, file_name: &str, pretty_print: bool) -> std::io::Result<()> {
        self.save_results_to_file_as_json_internal(file_name, &self.big_files, pretty_print)
    }
}

impl CommonData for BigFile {
    fn get_cd(&self) -> &CommonToolData {
        &self.common_data
    }
    fn get_cd_mut(&mut self) -> &mut CommonToolData {
        &mut self.common_data
    }
    fn found_any_broken_files(&self) -> bool {
        self.information.number_of_real_files > 0
    }
}

impl BigFile {
    pub const fn get_big_files(&self) -> &Vec<FileEntry> {
        &self.big_files
    }

    pub const fn get_information(&self) -> &Info {
        &self.information
    }

    pub(crate) fn get_params(&self) -> &BigFileParameters {
        &self.params
    }
}
