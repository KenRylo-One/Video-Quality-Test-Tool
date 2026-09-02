//! The comparison set: one reference, and one or more encodes.
//!
//! Every encode is measured against the same reference. The tool holds no idea of where a
//! file came from. A file that you pulled with `yt-dlp` is just another encode.

use crate::media::MediaInfo;
use crate::palette::SERIES_SLOTS;
use std::collections::BTreeMap;

/// The identity of one file inside one session.
///
/// The series color is keyed to this value and never to the row position, so a reorder
/// never repaints a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u64);

/// One file in the comparison set.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaFile {
    /// The identity of the file inside this session.
    pub id: FileId,
    /// What `ffprobe` reported.
    pub info: MediaInfo,
    /// The name to show. This starts as the file name.
    pub label: String,
}

/// What differs between one encode and the reference.
///
/// The interface draws a warn-colored mark after the value. The mark says that the tool
/// will correct the difference, and it is not a fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffMarks {
    /// The encode has a different frame size. The tool scales it up to match.
    pub resolution: bool,
    /// The encode has a different color range flag. The tool converts it to match.
    pub color_range: bool,
    /// The encode reports a different frame count. The tool measures the frames both
    /// files share.
    pub frame_count: bool,
}

impl DiffMarks {
    /// True when nothing differs.
    pub fn is_clear(self) -> bool {
        !self.resolution && !self.color_range && !self.frame_count
    }
}

/// One reference and its encodes.
#[derive(Debug, Clone, Default)]
pub struct ComparisonSet {
    files: BTreeMap<FileId, MediaFile>,
    order: Vec<FileId>,
    reference: Option<FileId>,
    slots: BTreeMap<FileId, usize>,
    next_id: u64,
}

impl ComparisonSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one file. The first file that arrives becomes the reference.
    pub fn add(&mut self, info: MediaInfo) -> FileId {
        let id = FileId(self.next_id);
        self.next_id += 1;
        let label = info.file_name();
        self.files.insert(id, MediaFile { id, info, label });
        self.order.push(id);
        if self.reference.is_none() {
            self.reference = Some(id);
        } else {
            self.assign_slot(id);
        }
        id
    }

    /// Removes one file.
    ///
    /// Removing means removing. The tool keeps no excluded row, because the run record
    /// already holds every file of every run that used it.
    pub fn remove(&mut self, id: FileId) {
        if self.reference == Some(id) {
            return;
        }
        self.files.remove(&id);
        self.order.retain(|entry| *entry != id);
        self.slots.remove(&id);
    }

    /// Makes one file the reference.
    ///
    /// The previous reference becomes a normal row in its old list position.
    pub fn promote_to_reference(&mut self, id: FileId) {
        if !self.files.contains_key(&id) || self.reference == Some(id) {
            return;
        }
        if let Some(old) = self.reference {
            self.assign_slot(old);
        }
        self.reference = Some(id);
        self.assign_slot(id);
    }

    /// Moves one file to the position of another. This is the drag handle.
    pub fn move_before(&mut self, moved: FileId, target: FileId) {
        if moved == target || !self.files.contains_key(&moved) {
            return;
        }
        self.order.retain(|entry| *entry != moved);
        match self.order.iter().position(|entry| *entry == target) {
            Some(index) => self.order.insert(index, moved),
            None => self.order.push(moved),
        }
    }

    /// The reference file.
    pub fn reference(&self) -> Option<&MediaFile> {
        self.reference.and_then(|id| self.files.get(&id))
    }

    /// The identity of the reference.
    pub fn reference_id(&self) -> Option<FileId> {
        self.reference
    }

    /// Every encode, in the order that the user set.
    pub fn encodes(&self) -> impl Iterator<Item = &MediaFile> {
        self.order
            .iter()
            .filter(move |id| Some(**id) != self.reference)
            .filter_map(move |id| self.files.get(id))
    }

    /// How many encodes the set holds.
    pub fn encode_count(&self) -> usize {
        self.order
            .iter()
            .filter(|id| Some(**id) != self.reference)
            .count()
    }

    /// How many files the set holds, the reference included.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when no file has arrived.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// One file.
    pub fn get(&self, id: FileId) -> Option<&MediaFile> {
        self.files.get(&id)
    }

    /// The palette slot of one encode, when it has one.
    pub fn slot_of(&self, id: FileId) -> Option<usize> {
        self.slots.get(&id).copied()
    }

    /// True when the set holds more encodes than the palette has slots.
    ///
    /// Above eight encodes the tool draws small multiples. It never reuses a color.
    pub fn exceeds_palette(&self) -> bool {
        self.encode_count() > SERIES_SLOTS
    }

    /// What differs between one encode and the reference.
    pub fn diff_marks(&self, id: FileId) -> DiffMarks {
        let (Some(reference), Some(file)) = (self.reference(), self.files.get(&id)) else {
            return DiffMarks::default();
        };
        if reference.id == id {
            return DiffMarks::default();
        }
        DiffMarks {
            resolution: crate::corrections::detect_resolution(&reference.info, &file.info, "")
                .is_some(),
            color_range: crate::corrections::detect_color_range(&reference.info, &file.info, "")
                .is_some(),
            frame_count: crate::corrections::detect_frame_count(&reference.info, &file.info, "")
                .correction
                .is_some(),
        }
    }

    /// Gives one file the lowest free palette slot, when it holds none.
    fn assign_slot(&mut self, id: FileId) {
        if self.slots.contains_key(&id) {
            return;
        }
        let taken: Vec<usize> = self.slots.values().copied().collect();
        if let Some(free) = (0..SERIES_SLOTS).find(|slot| !taken.contains(slot)) {
            self.slots.insert(id, free);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{ColorRange, Rational};
    use std::path::PathBuf;

    fn file(name: &str, width: u32, height: u32, range: ColorRange, pix_fmt: &str) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from(name),
            bytes: 1024,
            codec: "h264".into(),
            profile: None,
            width,
            height,
            pix_fmt: pix_fmt.into(),
            bit_depth: 8,
            color_range: range,
            color_space: Some("bt709".into()),
            frame_rate: Rational { num: 30, den: 1 },
            nb_frames: Some(150),
            duration_s: Some(5.0),
            bit_rate: Some(17076),
        }
    }

    #[test]
    fn the_first_file_becomes_the_reference() {
        let mut set = ComparisonSet::new();
        let first = set.add(file("a.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        let second = set.add(file("b.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        assert_eq!(set.reference_id(), Some(first));
        assert_eq!(set.encode_count(), 1);
        assert_eq!(set.encodes().next().unwrap().id, second);
    }

    #[test]
    fn four_files_give_four_rows_and_the_first_is_the_reference() {
        let mut set = ComparisonSet::new();
        let ids: Vec<_> = (0..4)
            .map(|index| {
                set.add(file(
                    &format!("f{index}.mp4"),
                    1920,
                    1080,
                    ColorRange::Tv,
                    "yuv420p",
                ))
            })
            .collect();
        assert_eq!(set.len(), 4);
        assert_eq!(set.encode_count(), 3);
        assert_eq!(set.reference_id(), Some(ids[0]));
    }

    #[test]
    fn promoting_puts_the_old_reference_back_in_its_place() {
        let mut set = ComparisonSet::new();
        let first = set.add(file("a.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        let second = set.add(file("b.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        let third = set.add(file("c.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));

        set.promote_to_reference(third);
        assert_eq!(set.reference_id(), Some(third));
        let order: Vec<_> = set.encodes().map(|encode| encode.id).collect();
        assert_eq!(order, vec![first, second]);
    }

    #[test]
    fn the_reference_cannot_be_removed() {
        let mut set = ComparisonSet::new();
        let first = set.add(file("a.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        set.remove(first);
        assert_eq!(set.reference_id(), Some(first));
    }

    #[test]
    fn a_color_slot_follows_the_file_and_not_the_row() {
        let mut set = ComparisonSet::new();
        set.add(file("ref.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        let first = set.add(file("a.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        let second = set.add(file("b.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));

        let before = (set.slot_of(first), set.slot_of(second));
        set.move_before(second, first);
        assert_eq!(before, (set.slot_of(first), set.slot_of(second)));
        assert_eq!(
            set.encodes().map(|e| e.id).collect::<Vec<_>>(),
            vec![second, first]
        );
    }

    #[test]
    fn removing_frees_the_slot_for_the_next_file() {
        let mut set = ComparisonSet::new();
        set.add(file("ref.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        let first = set.add(file("a.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        assert_eq!(set.slot_of(first), Some(0));
        set.remove(first);
        let next = set.add(file("c.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        assert_eq!(set.slot_of(next), Some(0));
    }

    #[test]
    fn a_diff_mark_appears_for_resolution_and_for_range() {
        let mut set = ComparisonSet::new();
        set.add(file("ref.mp4", 3840, 2160, ColorRange::Tv, "yuv420p"));
        let same = set.add(file("same.mp4", 3840, 2160, ColorRange::Tv, "yuv420p"));
        let smaller = set.add(file("small.mp4", 1920, 1080, ColorRange::Pc, "yuv420p"));

        assert!(set.diff_marks(same).is_clear());
        let marks = set.diff_marks(smaller);
        assert!(marks.resolution);
        assert!(marks.color_range);
    }

    #[test]
    fn a_jpeg_pixel_format_counts_as_full_range() {
        let mut set = ComparisonSet::new();
        set.add(file("ref.mp4", 512, 256, ColorRange::Tv, "yuv420p"));
        let jpeg = set.add(file("a.mp4", 512, 256, ColorRange::Unknown, "yuvj420p"));
        assert!(set.diff_marks(jpeg).color_range);
    }

    #[test]
    fn nine_encodes_exceed_the_palette() {
        let mut set = ComparisonSet::new();
        set.add(file("ref.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        for index in 0..8 {
            set.add(file(
                &format!("e{index}.mp4"),
                1920,
                1080,
                ColorRange::Tv,
                "yuv420p",
            ));
        }
        assert!(!set.exceeds_palette());
        set.add(file("e8.mp4", 1920, 1080, ColorRange::Tv, "yuv420p"));
        assert!(set.exceeds_palette());
    }
}
