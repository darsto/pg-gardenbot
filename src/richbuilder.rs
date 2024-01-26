use core::convert::TryFrom;
use core::ops::Range;

pub struct RichBuilder<'a> {
    text: String,
    formats: Vec<(Range<u32>, nwg::CharFormat)>,
    rich_label: &'a nwg::RichLabel,
}

impl<'a> RichBuilder<'a> {
    pub fn new(rich_label: &'a nwg::RichLabel) -> Self {
        Self {
            text: String::new(),
            formats: Vec::new(),
            rich_label,
        }
    }

    pub fn append(&mut self, text: &str, mut fmt: nwg::CharFormat) {
        fmt.height.get_or_insert(180);
        fmt.text_color.get_or_insert([0, 0, 0]);

        let text_start_idx = u32::try_from(self.text.len()).unwrap();
        let text_end_idx = text_start_idx + u32::try_from(text.len()).unwrap();

        self.text += text;
        self.formats.push((text_start_idx..text_end_idx, fmt));
    }
}

impl<'a> Drop for RichBuilder<'a> {
    fn drop(&mut self) {
        self.rich_label.set_text(&self.text);
        for (range, fmt) in self.formats.iter() {
            self.rich_label.set_char_format(range.clone(), fmt);
        }
    }
}
