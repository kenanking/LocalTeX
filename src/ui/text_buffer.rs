use std::ops::Range;

use gpui::{ClipboardItem, SharedString, UTF16Selection};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct TextBuffer {
    pub content: SharedString,
    pub selected_range: Range<usize>,
    pub selection_reversed: bool,
    pub marked_range: Option<Range<usize>>,
    pub is_selecting: bool,
}

impl TextBuffer {
    pub fn new() -> Self {
        Self {
            content: "".into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            is_selecting: false,
        }
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub fn move_to(&mut self, offset: usize) {
        self.selected_range = offset..offset;
    }

    pub fn select_to(&mut self, offset: usize) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
    }

    pub fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    pub fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    pub fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone())
    }

    pub fn replace_text(&mut self, range_utf16: Option<Range<usize>>, new_text: &str) {
        let range = self.resolve_range(range_utf16);
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        let end = range.start + new_text.len();
        self.selected_range = end..end;
        self.marked_range.take();
    }

    pub fn replace_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        let range = self.resolve_range(range_utf16);
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|selection| {
                let offset = |units| {
                    let mut count = 0;
                    let mut bytes = 0;
                    for ch in new_text.chars() {
                        if count >= units {
                            break;
                        }
                        count += ch.len_utf16();
                        bytes += ch.len_utf8();
                    }
                    range.start + bytes
                };
                offset(selection.start)..offset(selection.end)
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;
    }

    pub fn selected_text(&self) -> Option<String> {
        if self.selected_range.is_empty() {
            None
        } else {
            Some(self.content[self.selected_range.clone()].to_string())
        }
    }

    pub fn write_selection(&self, cx: &mut gpui::App) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn selected_utf16(&self) -> UTF16Selection {
        UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        }
    }

    pub fn text_for_utf16(
        &self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_selection_is_relative_to_inserted_text() {
        for (selection, text, caret, expected) in
            [(1..2, "X", 1, 2), (3..3, "中", 1, 6), (1..2, "😀中", 2, 5)]
        {
            let mut buffer = TextBuffer::new();
            buffer.content = "abc".into();
            buffer.selected_range = selection;
            buffer.replace_and_mark(None, text, Some(caret..caret));
            assert_eq!(buffer.selected_range, expected..expected);
            assert!(buffer.content.is_char_boundary(expected));
            buffer.replace_and_mark(None, "中文", Some(2..2));
            assert!(buffer.content.is_char_boundary(buffer.cursor_offset()));
            buffer.replace_text(None, "完成");
            assert!(buffer.marked_range.is_none());
        }
    }
}
