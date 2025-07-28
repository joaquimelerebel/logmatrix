use clap::{Parser, ValueEnum};
use rand::prelude::*;
use std::{
    collections::VecDeque,
    io,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{sleep, spawn},
    time::{Duration, Instant},
};
use terminal_size::{Height, Width, terminal_size};

#[derive(ValueEnum, Debug, Clone, Copy)] // ArgEnum here
#[clap(rename_all = "kebab_case")]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Default,
}

impl Color {
    fn to_ansi(&self) -> String {
        match self {
            Color::Default => format!("{esc}[0;0m", esc = 27 as char),
            Color::Black => format!("{esc}[0;30m", esc = 27 as char),
            Color::Red => format!("{esc}[0;31m", esc = 27 as char),
            Color::Cyan => format!("{esc}[0;36m", esc = 27 as char),
            Color::Magenta => format!("{esc}[0;35m", esc = 27 as char),
            Color::Yellow => format!("{esc}[0;33m", esc = 27 as char),
            Color::Blue => format!("{esc}[0;34m", esc = 27 as char),
            Color::White => format!("{esc}[0;37m", esc = 27 as char),
            Color::Green => format!("{esc}[0;32m", esc = 27 as char),
        }
    }
}

#[derive(ValueEnum, Debug, Clone)] // ArgEnum here
#[clap(rename_all = "kebab_case")]
enum Direction {
    Top,
    Bottom,
    SpiralRight,
}

#[derive(Parser, Clone)]
#[command(version, about, long_about = None)]
struct Args {
    #[clap(short, long, value_enum, default_value = "default")]
    /// color of the text... color can change due to themed terminal
    color: Color,
    #[clap(long, value_enum, default_value = "white")]
    /// highlight color of the text... color can change due to themed terminal
    highlight_color: Color,
    #[clap(long, value_enum, default_value = "3")]
    /// length of the highlight
    highlight_threshold: usize,
    #[clap(short, long, default_value = "100")]
    /// period between 2 refresh in ms
    frequency: u64,
    #[clap(short, long, value_enum, default_value = "bottom")]
    /// direction to which the logs will go
    direction: Direction,
    #[clap(short, long, default_value = "1")]
    /// spaces between 2 messages
    spaces: u16,
}

#[derive(Clone)]
struct CircularCharQueue {
    data: Vec<(char, Color)>,
    front_index: usize, // pointer to the watch head of the circular buffer
    back_index: usize,  //pointer to the head of the circular buffer
}

impl CircularCharQueue {
    fn new(size: usize) -> CircularCharQueue {
        CircularCharQueue {
            data: vec![(' ', Color::Default); size],
            front_index: size,
            back_index: 0,
        }
    }

    fn push_back(&mut self, n: char, c: Color) {
        self.data[self.back_index] = (n, c);

        self.back_index = if self.back_index == 0 {
            self.data.len() - 1
        } else {
            self.back_index - 1
        };

        self.front_index = self.back_index;
    }

    fn get_next(&mut self, direction: &Direction) -> (char, Color) {
        let cc = self.data[self.front_index];

        self.front_index = match direction {
            Direction::Top | Direction::SpiralRight => {
                if self.front_index == 0 {
                    self.data.len() - 1
                } else {
                    self.front_index - 1
                }
            }
            Direction::Bottom => {
                if self.front_index == self.data.len() - 1 {
                    0
                } else {
                    self.front_index + 1
                }
            }
        };

        cc
    }
}

#[derive(Clone)]
struct ColumnMat {
    invisible_cache: VecDeque<String>,
    visible_line: CircularCharQueue,
    index: usize, // index in the current invisible_cache
    color: Color,
    highlight: Color,
    highlight_threshold: usize,
}

impl ColumnMat {
    fn new(height: usize, color: Color, highlight: Color, highlight_threshold: usize) -> Self {
        ColumnMat {
            invisible_cache: VecDeque::new(),
            visible_line: CircularCharQueue::new(height),
            index: 0,
            color,
            highlight,
            highlight_threshold,
        }
    }

    fn add_line(&mut self, addon: String) {
        self.invisible_cache.push_back(addon);
    }

    fn tick(&mut self, spaces: u16) {
        if self.invisible_cache.is_empty() {
            self.visible_line.push_back(' ', Color::Default);
        } else if self.index == self.invisible_cache[0].len() {
            self.invisible_cache.pop_front();
            self.index = 0;
            for _ in 0..spaces {
                self.visible_line.push_back(' ', Color::Default);
            }
        } else {
            let a = self.invisible_cache[0].chars().nth(self.index).unwrap();
            if self.index < self.highlight_threshold {
                self.visible_line.push_back(a, self.highlight);
            } else {
                self.visible_line.push_back(a, self.color);
            }
            self.index += 1;
        };
    }

    fn get_next(&mut self, dir: &Direction) -> (char, Color) {
        self.visible_line.get_next(dir)
    }
}

struct Matrix {
    width: u16,
    height: u16,
    center_x: u16,
    center_y: u16,
    spiral_length: usize,
    columns: Vec<ColumnMat>,
    opt: Args,
    stdin_channel: Receiver<String>,
    rng: ThreadRng,
    spiral_coef: f32,
}

impl Matrix {
    fn new(opt: Args) -> Matrix {
        let (Width(width), Height(height)) = terminal_size().unwrap();
        let spiral_length = Matrix::get_spiral_length(height, width);
        let columns = Matrix::get_columns(width, height, spiral_length, &opt);
        let stdin_channel = Matrix::spawn_stdin_channel();
        let rng = rand::rng();
        let spiral_coef = 1500.;
        let (center_x, center_y) = ((width / 2), (height / 2));
        ctrlc::set_handler(Matrix::exit_matrix).expect("Error setting Ctrl-C handler");

        Matrix {
            width,
            height,
            center_x,
            center_y,
            spiral_length,
            columns,
            opt,
            rng,
            stdin_channel,
            spiral_coef,
        }
    }

    fn get_spiral_length(height: u16, width: u16) -> usize {
        ((height + width) * 2) as usize
    }

    fn get_columns(width: u16, height: u16, spiral_length: usize, opt: &Args) -> Vec<ColumnMat> {
        match opt.direction {
            Direction::SpiralRight => vec![
                ColumnMat::new(
                    spiral_length,
                    opt.color,
                    opt.highlight_color,
                    opt.highlight_threshold
                );
                1
            ],

            Direction::Top | Direction::Bottom => vec![
                ColumnMat::new(
                    height as usize,
                    opt.color,
                    opt.highlight_color,
                    opt.highlight_threshold
                );
                width as usize
            ],
        }
    }

    fn update_mat(&mut self) {
        let (Width(width), Height(height)) = terminal_size().unwrap();
        if self.width != width || self.height != height {
            self.height = height;
            self.width = width;

            (self.center_x, self.center_y) = ((self.width / 2), (self.height / 2));

            self.spiral_length = Matrix::get_spiral_length(height, width);
            self.columns = Matrix::get_columns(width, height, self.spiral_length, &self.opt);
            Matrix::clean_matrix();
        }
    }

    fn update_inputs(&mut self) -> Option<()> {
        let mut found_end = false;
        while !found_end {
            match self.stdin_channel.try_recv() {
                Ok(key) => {
                    let w_idx = (self.rng.random::<u16>() % self.columns.len() as u16) as usize;
                    self.columns[w_idx].add_line(key);
                }
                Err(TryRecvError::Empty) => found_end = true,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if !found_end {
            return None;
        }
        Some(())
    }

    fn spawn_stdin_channel() -> Receiver<String> {
        let (tx, rx) = mpsc::channel::<String>();
        spawn(move || {
            loop {
                let mut buffer = String::new();
                io::stdin().read_line(&mut buffer).unwrap();
                if buffer.is_empty() {
                    break;
                }
                let buffer = buffer.replace("\n", "");
                tx.send(buffer).unwrap();
            }
        });
        rx
    }

    fn spiral_exec(&mut self) {
        for i in 1..self.spiral_length {
            let (letter, color) = self.columns[0].get_next(&Direction::SpiralRight);
            let index = i as f32;
            let x = (self.r(index) * index.cos()).floor() as i16;
            let y = (self.r(index) * index.sin()).floor() as i16;
            let x_abs = (self.center_x as i16 + x) as u16;
            let y_abs = (self.center_y as i16 + y) as u16;

            if x_abs < self.width && y_abs < self.height {
                self.place_cursor(x_abs, y_abs);
                println!("{}{letter}{}", color.to_ansi(), Color::Default.to_ansi());
            }
        }
    }

    fn directional_exec(&mut self) {
        for _h in 0..self.height {
            let mut line = String::new();
            for col in self.columns.iter_mut() {
                let (letter, color) = col.get_next(&self.opt.direction);
                line += &format!("{}{letter}{}", color.to_ansi(), Color::Default.to_ansi());
            }
            println!("{line}{}", Color::Default.to_ansi());
        }
    }

    fn main_loop(&mut self) {
        let delta_t = Duration::from_millis(self.opt.frequency);
        Matrix::enter_matrix();
        loop {
            // update the size of window dynamically
            let now = Instant::now();
            self.update_mat();
            if self.update_inputs().is_none() {
                return;
            }

            for col in self.columns.iter_mut() {
                col.tick(self.opt.spaces);
            }

            self.place_cursor(1, 1);
            match self.opt.direction {
                Direction::SpiralRight => self.spiral_exec(),
                Direction::Top | Direction::Bottom => self.directional_exec(),
            };

            // speed limitation
            let elapsed_time = now.elapsed();
            let remaining_time = delta_t - elapsed_time;
            sleep(remaining_time);
        }
    }

    fn place_cursor(&self, x: u16, y: u16) {
        print!("{esc}[{y};{x}H", esc = 27 as char);
    }

    fn clean_matrix() {
        print!("{esc}[2J", esc = 27 as char)
    }
    fn enter_matrix() {
        print!("{esc}[?1049h", esc = 27 as char)
    }
    fn exit_matrix() {
        print!("{esc}[?1049l", esc = 27 as char)
    }

    // archimean spiral
    fn r(&mut self, angle: f32) -> f32 {
        self.spiral_coef / angle
    }
}

fn main() {
    let opt = Args::parse();
    Matrix::new(opt).main_loop();
}
