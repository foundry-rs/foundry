/// Word view widget
pub mod word {
    use std::collections::HashMap;
    use tui::{
        buffer::Buffer,
        layout::Rect,
        style::{Color, Modifier, Style},
        text::{Span, Spans},
        widgets::{Block, Paragraph, StatefulWidget, Widget, Wrap},
    };

    pub struct WordView<'a> {
        /// The words to display
        buffer: &'a [u8],
        /// The number of bytes in the buffer
        len: usize,
        /// Words (32 bytes) to highlight
        highlights: HashMap<usize, Color>,
        /// Word labels
        labels: HashMap<usize, String>,
        /// An optional wrapping [Block]
        block: Option<Block<'a>>,
        /// The item number mode determines how items are counted, either per word or per byte.
        item_number_mode: ItemNumberMode,
        /// The item number format determines how item numbers are displayed, either decimal or
        /// hexadecimal.
        item_number_format: ItemNumberFormat,
    }

    impl<'a> WordView<'a> {
        pub fn new(buffer: &'a [u8], len: usize) -> Self {
            Self {
                buffer,
                len,
                highlights: HashMap::new(),
                labels: HashMap::new(),
                block: None,
                item_number_mode: ItemNumberMode::Word,
                item_number_format: ItemNumberFormat::Hexadecimal,
            }
        }

        /// Wrap the view in a block.
        pub fn block(mut self, block: Block<'a>) -> Self {
            self.block = Some(block);
            self
        }

        /// Set the way items are counted.
        pub fn item_number_mode(mut self, mode: ItemNumberMode) -> Self {
            self.item_number_mode = mode;
            self
        }

        /// Set the way item numbers are displayed.
        pub fn item_number_format(mut self, format: ItemNumberFormat) -> Self {
            self.item_number_format = format;
            self
        }

        /// Highlight a word with a color.
        ///
        /// A word is 32 bytes.
        pub fn highlight(mut self, word: usize, color: Color) -> Self {
            self.highlights.insert(word, color);
            self
        }

        /// Label a word.
        ///
        /// A word is 32 bytes.
        pub fn label(mut self, word: usize, label: String) -> Self {
            self.labels.insert(word, label);
            self
        }
    }

    #[derive(Default)]
    pub struct WordViewState {
        /// The first word (32 bytes) in the buffer to display
        start: usize,
        /// The current view mode
        mode: ViewMode,
    }

    impl WordViewState {
        /// Scroll to the first word.
        pub fn scroll_to_top(&mut self) {
            self.start = 0;
        }

        /// Scroll down one word.
        pub fn scroll_down(&mut self) {
            self.start = self.start.saturating_add(1);
        }

        /// Scroll up one word.
        pub fn scroll_up(&mut self) {
            self.start = self.start.saturating_sub(1);
        }

        /// Toggle the view mode between two modes.
        pub fn toggle_mode(&mut self, a: ViewMode, b: ViewMode) {
            if self.mode == a {
                self.mode = b;
            } else {
                self.mode = a;
            }
        }
    }

    impl<'a> StatefulWidget for WordView<'a> {
        type State = WordViewState;

        fn render(mut self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
            // TODO: Wrap out of bounds `state.start` between 0 and `self.len`
            let area = match self.block.take() {
                Some(b) => {
                    let inner_area = b.inner(area);
                    b.render(area, buf);
                    inner_area
                }
                None => area,
            };

            let mut graphemes = Vec::new();
            for (i, word) in self.buffer.chunks(32).enumerate().skip(state.start) {
                // Format word
                let color = self.highlights.get(&i);
                let bytes: Vec<Span> = word
                    .iter()
                    .map(|byte| {
                        Span::styled(
                            format!("{:02x} ", byte),
                            if let Some(color) = color {
                                Style::default().fg(*color)
                            } else if *byte == 0 {
                                Style::default().add_modifier(Modifier::DIM)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        )
                    })
                    .collect();

                // Add item number
                let min_len = match self.item_number_format {
                    ItemNumberFormat::Hexadecimal => {
                        format!("{:x}", self.len)
                    }
                    ItemNumberFormat::Decimal => self.len.to_string(),
                }
                .len();
                let item_number = match self.item_number_format {
                    ItemNumberFormat::Hexadecimal => {
                        format!(
                            "{:0min_len$x}| ",
                            i * self.item_number_mode.scalar(),
                            min_len = min_len
                        )
                    }
                    ItemNumberFormat::Decimal => {
                        format!(
                            "{:0min_len$}| ",
                            i * self.item_number_mode.scalar(),
                            min_len = min_len
                        )
                    }
                };
                let mut spans = vec![Span::styled(item_number, Style::default().fg(Color::White))];
                spans.extend(bytes);

                // TODO: Label mode
                match state.mode {
                    ViewMode::Utf8 => {
                        let chars: Vec<Span> = word
                            .chunks(4)
                            .map(|utf| {
                                if let Ok(s) = std::str::from_utf8(utf) {
                                    Span::raw(s.replace(char::from(0), "."))
                                } else {
                                    Span::raw(".")
                                }
                            })
                            .collect();
                        spans.push(Span::raw("|"));
                        spans.extend(chars);
                    }
                    ViewMode::Label => {
                        if let Some(label) = self.labels.get(&i) {
                            spans.push(Span::raw(format!("| {label}")));
                        } else {
                            spans.push(Span::raw("| ".to_string()));
                        }
                    }
                    _ => (),
                }

                graphemes.push(Spans::from(spans));
            }

            Paragraph::new(graphemes).wrap(Wrap { trim: true }).render(area, buf);
        }
    }

    /// The view mode for memory words.
    #[derive(Default, Eq, PartialEq)]
    pub enum ViewMode {
        /// Just displays the words
        #[default]
        None,
        /// Display words as UTF8
        Utf8,
        /// Display words with their labels
        Label,
    }

    #[derive(Default, Eq, PartialEq)]
    pub enum ItemNumberMode {
        /// Count items by words
        #[default]
        Word,
        /// Count items by bytes
        Byte,
    }

    impl ItemNumberMode {
        pub fn scalar(&self) -> usize {
            match self {
                ItemNumberMode::Word => 32,
                ItemNumberMode::Byte => 1,
            }
        }
    }

    #[derive(Default, Eq, PartialEq)]
    pub enum ItemNumberFormat {
        /// Count items by words
        #[default]
        Hexadecimal,
        /// Count items by bytes
        Decimal,
    }
}
