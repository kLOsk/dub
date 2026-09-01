//! Configurable browser columns (PRD §8.5.3.1).
//!
//! The column *picker* shipped with the library rounds over the fixed
//! set of fields [`crate::TrackRow`] already carried. This module is the
//! other half — the columns whose data is not in that fixed row:
//! per-source metadata verbatim, Dub's own analysis outputs alongside the
//! active ones, audio-file facts, and aggregated mix history.
//!
//! Three rules shape the design:
//!
//! * **Ids are stable strings.** The Apple shell persists the user's
//!   column set in preferences; a rename here would silently reset
//!   everyone's browser layout, so [`LibraryColumnId::as_str`] is a
//!   contract, not a display detail (labels are separate and free to
//!   change).
//! * **A disabled column costs nothing.** The SELECT and its joins are
//!   generated from the active [`ColumnSet`], so a user who never turns
//!   on the mix-history group never pays for the aggregate.
//! * **User input never reaches SQL.** Callers hand over parsed
//!   [`LibraryColumnId`] values; the SQL text for each one lives here and
//!   nowhere else, exactly as [`crate::TrackSortKey`] does for ORDER BY.

use rusqlite::Row;

/// A metadata origin that can be shown verbatim in its own columns.
///
/// These are the six values `track_metadata_source.source` permits.
/// PRD §8.5.3.1 also lists `mixedinkey`; Mixed In Key writes through the
/// id3 comment field rather than a source row of its own, so there is
/// nothing to select until an importer mints one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataSource {
    /// Parsed from the filename (`crate::parse_filename`).
    Filename,
    /// ID3 / Vorbis tags read off the file itself.
    Id3,
    /// Serato (database V2 + GEOB frames).
    Serato,
    /// Traktor `collection.nml`.
    Traktor,
    /// rekordbox XML export.
    Rekordbox,
    /// iTunes / Apple Music `Library.xml`.
    ITunes,
}

impl MetadataSource {
    /// Every source, in the order the picker lists them.
    pub const ALL: [MetadataSource; 6] = [
        MetadataSource::Filename,
        MetadataSource::Id3,
        MetadataSource::Serato,
        MetadataSource::Traktor,
        MetadataSource::Rekordbox,
        MetadataSource::ITunes,
    ];

    /// The `track_metadata_source.source` string, and the id prefix.
    pub fn as_str(self) -> &'static str {
        match self {
            MetadataSource::Filename => "filename",
            MetadataSource::Id3 => "id3",
            MetadataSource::Serato => "serato",
            MetadataSource::Traktor => "traktor",
            MetadataSource::Rekordbox => "rekordbox",
            MetadataSource::ITunes => "itunes",
        }
    }

    /// Display name for the picker group and column headers.
    pub fn label(self) -> &'static str {
        match self {
            MetadataSource::Filename => "Filename",
            MetadataSource::Id3 => "ID3",
            MetadataSource::Serato => "Serato",
            MetadataSource::Traktor => "Traktor",
            MetadataSource::Rekordbox => "rekordbox",
            MetadataSource::ITunes => "iTunes",
        }
    }

    /// SQL alias this source is already joined under in the fixed
    /// track SELECT. Per-source columns therefore add no joins at all —
    /// the §8.1 priority chain needed every source joined anyway.
    fn alias(self) -> &'static str {
        match self {
            MetadataSource::Filename => "fn",
            MetadataSource::Id3 => "i3",
            MetadataSource::Serato => "sr",
            MetadataSource::Traktor => "tr",
            MetadataSource::Rekordbox => "rb",
            MetadataSource::ITunes => "it",
        }
    }

    /// Fields this source can actually carry. The filename parser
    /// recovers only what a DJ-rip filename encodes; the tag-backed
    /// sources carry the full set. Absent combinations are absent from
    /// the registry rather than showing a column that is always empty.
    fn fields(self) -> &'static [MetadataField] {
        use MetadataField::*;
        match self {
            MetadataSource::Filename => &[Title, Artist, Year, Version],
            MetadataSource::Id3 => &[Title, Artist, Album, Comment, Bpm, Key, Year, Version],
            _ => &[Title, Artist, Album, Comment, Bpm, Key, Year],
        }
    }
}

/// A field of a per-source metadata row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataField {
    /// `title`.
    Title,
    /// `artist`.
    Artist,
    /// `album`.
    Album,
    /// `comment`.
    Comment,
    /// `bpm` — the source's own tempo claim, not a beat grid.
    Bpm,
    /// `key` in the source's own notation.
    Key,
    /// `year`.
    Year,
    /// `version_token` — the canonical remix / edit tokens.
    Version,
}

impl MetadataField {
    /// Id suffix, e.g. the `bpm` of `serato_bpm`.
    fn as_str(self) -> &'static str {
        match self {
            MetadataField::Title => "title",
            MetadataField::Artist => "artist",
            MetadataField::Album => "album",
            MetadataField::Comment => "comment",
            MetadataField::Bpm => "bpm",
            MetadataField::Key => "key",
            MetadataField::Year => "year",
            MetadataField::Version => "version",
        }
    }

    /// Header label suffix, e.g. the `BPM` of `Serato BPM`.
    fn label(self) -> &'static str {
        match self {
            MetadataField::Title => "Title",
            MetadataField::Artist => "Artist",
            MetadataField::Album => "Album",
            MetadataField::Comment => "Comment",
            MetadataField::Bpm => "BPM",
            MetadataField::Key => "Key",
            MetadataField::Year => "Year",
            MetadataField::Version => "Version",
        }
    }

    /// Column name on `track_metadata_source`.
    fn sql_column(self) -> &'static str {
        match self {
            MetadataField::Version => "version_token",
            other => other.as_str(),
        }
    }

    fn kind(self) -> ColumnKind {
        match self {
            MetadataField::Bpm => ColumnKind::Bpm,
            MetadataField::Year => ColumnKind::Integer,
            _ => ColumnKind::Text,
        }
    }
}

/// Picker grouping. Mirrors the table in PRD §8.5.3.1 so the Apple
/// shell can build the grouped context menu straight off the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnGroup {
    /// Identity / library facts about the row itself.
    Library,
    /// Dub's own analysis outputs.
    Analysis,
    /// One source's metadata, verbatim.
    PerSource(MetadataSource),
    /// Facts about the audio file on disk.
    AudioFile,
    /// Aggregates over `play_history`.
    MixHistory,
}

impl ColumnGroup {
    /// Display name for the picker's section header.
    pub fn label(self) -> String {
        match self {
            ColumnGroup::Library => "Library".to_string(),
            ColumnGroup::Analysis => "Analysis".to_string(),
            ColumnGroup::PerSource(source) => source.label().to_string(),
            ColumnGroup::AudioFile => "Audio file".to_string(),
            ColumnGroup::MixHistory => "Mix history".to_string(),
        }
    }
}

/// How a cell's value should be read back and rendered.
///
/// The kind lives in the registry rather than in each value so the
/// renderer formats an absent BPM the same way it formats a present
/// one, and so a numeric column sorts numerically even where every
/// visible row happens to be empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnKind {
    /// Free text.
    Text,
    /// Whole number.
    Integer,
    /// Decimal, rendered to one place.
    Real,
    /// Tempo, rendered to two places.
    Bpm,
    /// Unix **seconds**. Millisecond sources are divided in SQL so the
    /// renderer never has to know which table a column came from.
    Timestamp,
    /// Byte count.
    Bytes,
    /// Yes / no.
    Flag,
}

/// One configurable column, addressed by a stable id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryColumnId {
    /// `tracks.created_at`.
    DateAdded,
    /// Comma-joined names of the Dub crates holding this track.
    InCrates,
    /// Whether §8.1 dedupe linked a sibling version.
    Duplicate,
    /// Whether the primary file is flagged missing (§8.5.5).
    Missing,
    /// One field of one source's metadata row, verbatim.
    PerSource(MetadataSource, MetadataField),
    /// Dub's analyser's own BPM, next to the active grid's.
    BpmAuto,
    /// Dub's analyser's own key, next to the active one.
    KeyAuto,
    /// Integrated loudness (LUFS-I).
    LufsI,
    /// True peak (dBTP).
    TruePeak,
    /// Grid + loudness + waveform all cached (PRD §8.3).
    Prepared,
    /// Container / codec of the primary file.
    Codec,
    /// Sample rate in Hz.
    SampleRate,
    /// Bit depth.
    BitDepth,
    /// Channel count.
    ChannelCount,
    /// File size in bytes.
    FileSize,
    /// Absolute path when the volume is mounted, else the relative one.
    FilePath,
    /// File mtime.
    FileModified,
    /// Lifetime `play_start` count.
    PlayCount,
    /// Most recent `play_start`.
    LastPlayed,
    /// Most recent `load`.
    LastLoaded,
    /// `play_start` count in the last seven days.
    PlayedLast7d,
}

/// An extra join a column needs. The fixed track SELECT already joins
/// every metadata source, the active grid, the active key, the analysis
/// cache and the primary file, so only these four are ever added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ColumnJoin {
    AutoGrid,
    AutoKey,
    Crates,
    History,
}

impl ColumnJoin {
    fn sql(self) -> &'static str {
        match self {
            ColumnJoin::AutoGrid => {
                " LEFT JOIN track_beatgrids xg \
                   ON xg.track_id = t.id AND xg.source = 'auto'"
            }
            ColumnJoin::AutoKey => {
                " LEFT JOIN track_keys xk \
                   ON xk.track_id = t.id AND xk.source = 'auto'"
            }
            ColumnJoin::Crates => {
                " LEFT JOIN ( \
                     SELECT ct.track_id AS track_id, \
                            GROUP_CONCAT(c.name, ', ') AS names \
                     FROM crate_tracks ct \
                     JOIN crates c ON c.id = ct.crate_id \
                     GROUP BY ct.track_id \
                   ) xc ON xc.track_id = t.id"
            }
            // `played_7d` reads SQLite's clock rather than a bound
            // parameter so adding a mix-history column never renumbers
            // the positional params every caller already passes.
            ColumnJoin::History => {
                " LEFT JOIN ( \
                     SELECT track_id AS track_id, \
                            SUM(event_type = 'play_start') AS play_count, \
                            MAX(CASE WHEN event_type = 'play_start' \
                                     THEN timestamp_ms END) AS last_played, \
                            MAX(CASE WHEN event_type = 'load' \
                                     THEN timestamp_ms END) AS last_loaded, \
                            SUM(event_type = 'play_start' AND timestamp_ms >= \
                                (strftime('%s', 'now') - 604800) * 1000) AS played_7d \
                     FROM play_history \
                     GROUP BY track_id \
                   ) xh ON xh.track_id = t.id"
            }
        }
    }
}

impl LibraryColumnId {
    /// Every column the picker can offer, in picker order.
    pub fn all() -> Vec<LibraryColumnId> {
        let mut out = vec![
            LibraryColumnId::DateAdded,
            LibraryColumnId::InCrates,
            LibraryColumnId::Duplicate,
            LibraryColumnId::Missing,
            LibraryColumnId::BpmAuto,
            LibraryColumnId::KeyAuto,
            LibraryColumnId::LufsI,
            LibraryColumnId::TruePeak,
            LibraryColumnId::Prepared,
        ];
        for source in MetadataSource::ALL {
            for field in source.fields() {
                out.push(LibraryColumnId::PerSource(source, *field));
            }
        }
        out.extend([
            LibraryColumnId::Codec,
            LibraryColumnId::SampleRate,
            LibraryColumnId::BitDepth,
            LibraryColumnId::ChannelCount,
            LibraryColumnId::FileSize,
            LibraryColumnId::FilePath,
            LibraryColumnId::FileModified,
            LibraryColumnId::PlayCount,
            LibraryColumnId::LastPlayed,
            LibraryColumnId::LastLoaded,
            LibraryColumnId::PlayedLast7d,
        ]);
        out
    }

    /// The stable string id. Persisted in the Apple shell's
    /// preferences — treat a change here as a schema change.
    pub fn as_str(self) -> String {
        match self {
            LibraryColumnId::DateAdded => "date_added".to_string(),
            LibraryColumnId::InCrates => "in_crates".to_string(),
            LibraryColumnId::Duplicate => "duplicates".to_string(),
            LibraryColumnId::Missing => "missing".to_string(),
            LibraryColumnId::PerSource(source, field) => {
                format!("{}_{}", source.as_str(), field.as_str())
            }
            LibraryColumnId::BpmAuto => "bpm_auto".to_string(),
            LibraryColumnId::KeyAuto => "key_auto".to_string(),
            LibraryColumnId::LufsI => "lufs_i".to_string(),
            LibraryColumnId::TruePeak => "true_peak".to_string(),
            LibraryColumnId::Prepared => "prepared".to_string(),
            LibraryColumnId::Codec => "codec".to_string(),
            LibraryColumnId::SampleRate => "sample_rate".to_string(),
            LibraryColumnId::BitDepth => "bit_depth".to_string(),
            LibraryColumnId::ChannelCount => "channel_count".to_string(),
            LibraryColumnId::FileSize => "file_size".to_string(),
            LibraryColumnId::FilePath => "file_path".to_string(),
            LibraryColumnId::FileModified => "file_modified".to_string(),
            LibraryColumnId::PlayCount => "play_count".to_string(),
            LibraryColumnId::LastPlayed => "last_played".to_string(),
            LibraryColumnId::LastLoaded => "last_loaded".to_string(),
            LibraryColumnId::PlayedLast7d => "played_last_7d".to_string(),
        }
    }

    /// Parse a persisted id back. Unknown ids return `None` rather than
    /// erroring: a preferences file written by a newer build must
    /// degrade to "that column is gone", never to a failed listing.
    pub fn from_id(id: &str) -> Option<LibraryColumnId> {
        LibraryColumnId::all()
            .into_iter()
            .find(|c| c.as_str() == id)
    }

    /// Picker group.
    pub fn group(self) -> ColumnGroup {
        match self {
            LibraryColumnId::DateAdded
            | LibraryColumnId::InCrates
            | LibraryColumnId::Duplicate
            | LibraryColumnId::Missing => ColumnGroup::Library,
            LibraryColumnId::PerSource(source, _) => ColumnGroup::PerSource(source),
            LibraryColumnId::BpmAuto
            | LibraryColumnId::KeyAuto
            | LibraryColumnId::LufsI
            | LibraryColumnId::TruePeak
            | LibraryColumnId::Prepared => ColumnGroup::Analysis,
            LibraryColumnId::Codec
            | LibraryColumnId::SampleRate
            | LibraryColumnId::BitDepth
            | LibraryColumnId::ChannelCount
            | LibraryColumnId::FileSize
            | LibraryColumnId::FilePath
            | LibraryColumnId::FileModified => ColumnGroup::AudioFile,
            LibraryColumnId::PlayCount
            | LibraryColumnId::LastPlayed
            | LibraryColumnId::LastLoaded
            | LibraryColumnId::PlayedLast7d => ColumnGroup::MixHistory,
        }
    }

    /// Column header text.
    pub fn label(self) -> String {
        match self {
            LibraryColumnId::DateAdded => "Added".to_string(),
            LibraryColumnId::InCrates => "Crates".to_string(),
            LibraryColumnId::Duplicate => "Dupe".to_string(),
            LibraryColumnId::Missing => "Missing".to_string(),
            LibraryColumnId::PerSource(source, field) => {
                format!("{} {}", source.label(), field.label())
            }
            LibraryColumnId::BpmAuto => "BPM (auto)".to_string(),
            LibraryColumnId::KeyAuto => "Key (auto)".to_string(),
            LibraryColumnId::LufsI => "LUFS-I".to_string(),
            LibraryColumnId::TruePeak => "True peak".to_string(),
            LibraryColumnId::Prepared => "Prepared".to_string(),
            LibraryColumnId::Codec => "Codec".to_string(),
            LibraryColumnId::SampleRate => "Rate".to_string(),
            LibraryColumnId::BitDepth => "Depth".to_string(),
            LibraryColumnId::ChannelCount => "Channels".to_string(),
            LibraryColumnId::FileSize => "Size".to_string(),
            LibraryColumnId::FilePath => "Path".to_string(),
            LibraryColumnId::FileModified => "Modified".to_string(),
            LibraryColumnId::PlayCount => "Plays".to_string(),
            LibraryColumnId::LastPlayed => "Last played".to_string(),
            LibraryColumnId::LastLoaded => "Last loaded".to_string(),
            LibraryColumnId::PlayedLast7d => "Plays (7d)".to_string(),
        }
    }

    /// Value kind, for reading the cell back and rendering it.
    pub fn kind(self) -> ColumnKind {
        match self {
            LibraryColumnId::DateAdded
            | LibraryColumnId::FileModified
            | LibraryColumnId::LastPlayed
            | LibraryColumnId::LastLoaded => ColumnKind::Timestamp,
            LibraryColumnId::InCrates
            | LibraryColumnId::KeyAuto
            | LibraryColumnId::Codec
            | LibraryColumnId::FilePath => ColumnKind::Text,
            LibraryColumnId::Duplicate | LibraryColumnId::Missing | LibraryColumnId::Prepared => {
                ColumnKind::Flag
            }
            LibraryColumnId::PerSource(_, field) => field.kind(),
            LibraryColumnId::BpmAuto => ColumnKind::Bpm,
            LibraryColumnId::LufsI | LibraryColumnId::TruePeak => ColumnKind::Real,
            LibraryColumnId::SampleRate
            | LibraryColumnId::BitDepth
            | LibraryColumnId::ChannelCount
            | LibraryColumnId::PlayCount
            | LibraryColumnId::PlayedLast7d => ColumnKind::Integer,
            LibraryColumnId::FileSize => ColumnKind::Bytes,
        }
    }

    /// SQL expression producing this column's value.
    fn sql(self) -> String {
        match self {
            LibraryColumnId::DateAdded => "t.created_at".to_string(),
            LibraryColumnId::InCrates => "xc.names".to_string(),
            LibraryColumnId::Duplicate => {
                "CASE WHEN t.duplicate_link_track_id IS NOT NULL THEN 1 ELSE 0 END".to_string()
            }
            LibraryColumnId::Missing => "COALESCE(pf.is_missing, 0)".to_string(),
            LibraryColumnId::PerSource(source, field) => {
                format!("{}.{}", source.alias(), field.sql_column())
            }
            LibraryColumnId::BpmAuto => "xg.bpm".to_string(),
            LibraryColumnId::KeyAuto => "xk.key_notation".to_string(),
            LibraryColumnId::LufsI => "ac.lufs_i".to_string(),
            LibraryColumnId::TruePeak => "ac.true_peak_dbtp".to_string(),
            LibraryColumnId::Prepared => "CASE WHEN ac.has_active_grid = 1 \
                 AND ac.has_lufs = 1 AND ac.has_waveform = 1 THEN 1 ELSE 0 END"
                .to_string(),
            LibraryColumnId::Codec => "pf.codec".to_string(),
            LibraryColumnId::SampleRate => "pf.sample_rate".to_string(),
            LibraryColumnId::BitDepth => "pf.bit_depth".to_string(),
            LibraryColumnId::ChannelCount => "pf.channel_count".to_string(),
            LibraryColumnId::FileSize => "pf.file_size".to_string(),
            LibraryColumnId::FilePath => "COALESCE(( \
                     SELECT v.last_known_mount_point || '/' || pf.relative_path \
                     FROM volumes v WHERE v.volume_uuid = pf.volume_uuid \
                 ), pf.relative_path)"
                .to_string(),
            LibraryColumnId::FileModified => "pf.mtime".to_string(),
            LibraryColumnId::PlayCount => "COALESCE(xh.play_count, 0)".to_string(),
            LibraryColumnId::LastPlayed => "xh.last_played / 1000".to_string(),
            LibraryColumnId::LastLoaded => "xh.last_loaded / 1000".to_string(),
            LibraryColumnId::PlayedLast7d => "COALESCE(xh.played_7d, 0)".to_string(),
        }
    }

    fn join(self) -> Option<ColumnJoin> {
        match self {
            LibraryColumnId::InCrates => Some(ColumnJoin::Crates),
            LibraryColumnId::BpmAuto => Some(ColumnJoin::AutoGrid),
            LibraryColumnId::KeyAuto => Some(ColumnJoin::AutoKey),
            LibraryColumnId::PlayCount
            | LibraryColumnId::LastPlayed
            | LibraryColumnId::LastLoaded
            | LibraryColumnId::PlayedLast7d => Some(ColumnJoin::History),
            _ => None,
        }
    }

    /// Extra `track_files` column this one needs inside the
    /// primary-file subquery.
    fn primary_file_column(self) -> Option<&'static str> {
        match self {
            LibraryColumnId::Missing => Some("is_missing"),
            LibraryColumnId::Codec => Some("codec"),
            LibraryColumnId::SampleRate => Some("sample_rate"),
            LibraryColumnId::BitDepth => Some("bit_depth"),
            LibraryColumnId::ChannelCount => Some("channel_count"),
            LibraryColumnId::FileSize => Some("file_size"),
            LibraryColumnId::FileModified => Some("mtime"),
            _ => None,
        }
    }
}

/// One cell of a configurable column.
///
/// `Empty` is distinct from `Text("")` on purpose: the browser renders
/// an em-dash for "no value recorded" and an empty cell for "the source
/// recorded an empty string", and the difference is exactly what the
/// per-source disagreement view exists to show.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnValue {
    /// No value.
    Empty,
    /// Text.
    Text(String),
    /// Whole number.
    Int(i64),
    /// Decimal.
    Real(f64),
    /// Yes / no.
    Flag(bool),
}

/// The user's active set of configurable columns, in display order.
///
/// Duplicates are collapsed on construction so a caller cannot make the
/// SELECT quadratic by handing over the same id twice.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColumnSet {
    columns: Vec<LibraryColumnId>,
}

impl ColumnSet {
    /// Build from an ordered iterator, keeping first occurrences.
    pub fn new(columns: impl IntoIterator<Item = LibraryColumnId>) -> Self {
        let mut out: Vec<LibraryColumnId> = Vec::new();
        for column in columns {
            if !out.contains(&column) {
                out.push(column);
            }
        }
        ColumnSet { columns: out }
    }

    /// The columns, in the order their cells are returned.
    pub fn columns(&self) -> &[LibraryColumnId] {
        &self.columns
    }

    /// `true` when no configurable column is switched on — the case
    /// that must generate exactly the SQL the browser used before this
    /// module existed.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// `, <expr> AS x0, <expr> AS x1 …` appended to the fixed SELECT
    /// list. Empty when no column is active.
    pub(crate) fn select_fragment(&self) -> String {
        let mut out = String::new();
        for (index, column) in self.columns.iter().enumerate() {
            out.push_str(&format!(", {} AS x{index}", column.sql()));
        }
        out
    }

    /// The joins the active columns need, deduplicated and in a stable
    /// order so two sets with the same members prepare the same SQL and
    /// hit the statement cache.
    pub(crate) fn join_fragment(&self) -> String {
        let mut joins: Vec<ColumnJoin> = self.columns.iter().filter_map(|c| c.join()).collect();
        joins.sort_unstable();
        joins.dedup();
        joins.into_iter().map(ColumnJoin::sql).collect()
    }

    /// Extra `track_files` columns the primary-file subquery must
    /// project, deduplicated and ordered.
    pub(crate) fn primary_file_columns(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self
            .columns
            .iter()
            .filter_map(|c| c.primary_file_column())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Read this set's cells out of a row whose fixed columns end at
    /// `first_index`.
    pub(crate) fn read_row(
        &self,
        row: &Row<'_>,
        first_index: usize,
    ) -> rusqlite::Result<Vec<ColumnValue>> {
        self.columns
            .iter()
            .enumerate()
            .map(|(offset, column)| {
                let index = first_index + offset;
                Ok(match column.kind() {
                    ColumnKind::Text => match row.get::<_, Option<String>>(index)? {
                        Some(text) => ColumnValue::Text(text),
                        None => ColumnValue::Empty,
                    },
                    ColumnKind::Integer | ColumnKind::Bytes | ColumnKind::Timestamp => {
                        match row.get::<_, Option<i64>>(index)? {
                            Some(number) => ColumnValue::Int(number),
                            None => ColumnValue::Empty,
                        }
                    }
                    ColumnKind::Real | ColumnKind::Bpm => match row.get::<_, Option<f64>>(index)? {
                        Some(number) => ColumnValue::Real(number),
                        None => ColumnValue::Empty,
                    },
                    ColumnKind::Flag => match row.get::<_, Option<i64>>(index)? {
                        Some(flag) => ColumnValue::Flag(flag != 0),
                        None => ColumnValue::Empty,
                    },
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_across_the_registry() {
        let mut ids: Vec<String> = LibraryColumnId::all().iter().map(|c| c.as_str()).collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "every column id must be distinct");
    }

    /// The ids are persisted in the user's preferences, so a rename
    /// silently resets everyone's browser layout. This test is the
    /// tripwire.
    #[test]
    fn every_id_round_trips_through_from_id() {
        for column in LibraryColumnId::all() {
            let id = column.as_str();
            assert_eq!(
                LibraryColumnId::from_id(&id),
                Some(column),
                "id {id} must parse back to itself"
            );
        }
    }

    #[test]
    fn unknown_id_is_none_rather_than_an_error() {
        assert_eq!(LibraryColumnId::from_id("serato_energy"), None);
        assert_eq!(LibraryColumnId::from_id(""), None);
    }

    #[test]
    fn per_source_ids_read_the_way_the_prd_writes_them() {
        assert_eq!(
            LibraryColumnId::PerSource(MetadataSource::Serato, MetadataField::Bpm).as_str(),
            "serato_bpm"
        );
        assert_eq!(
            LibraryColumnId::PerSource(MetadataSource::Traktor, MetadataField::Key).as_str(),
            "traktor_key"
        );
        assert_eq!(
            LibraryColumnId::PerSource(MetadataSource::Filename, MetadataField::Version).as_str(),
            "filename_version"
        );
    }

    /// The filename parser recovers title / artist / year / version and
    /// nothing else; offering a "Filename BPM" column would show an
    /// always-empty column and teach the user to distrust the group.
    #[test]
    fn filename_source_offers_only_what_a_filename_carries() {
        let ids: Vec<String> = LibraryColumnId::all()
            .iter()
            .filter(|c| c.group() == ColumnGroup::PerSource(MetadataSource::Filename))
            .map(|c| c.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                "filename_title",
                "filename_artist",
                "filename_year",
                "filename_version"
            ]
        );
    }

    #[test]
    fn an_empty_set_generates_no_sql_at_all() {
        let set = ColumnSet::default();
        assert!(set.is_empty());
        assert_eq!(set.select_fragment(), "");
        assert_eq!(set.join_fragment(), "");
        assert!(set.primary_file_columns().is_empty());
    }

    /// The §8.1 priority chain already joins every metadata source, so
    /// the per-source group — the migration-trust feature — is free.
    #[test]
    fn per_source_columns_add_no_joins() {
        let set = ColumnSet::new([
            LibraryColumnId::PerSource(MetadataSource::Serato, MetadataField::Bpm),
            LibraryColumnId::PerSource(MetadataSource::Rekordbox, MetadataField::Key),
            LibraryColumnId::PerSource(MetadataSource::Id3, MetadataField::Comment),
        ]);
        assert_eq!(set.join_fragment(), "");
        assert_eq!(
            set.select_fragment(),
            ", sr.bpm AS x0, rb.key AS x1, i3.comment AS x2"
        );
    }

    #[test]
    fn the_mix_history_group_shares_one_aggregate() {
        let set = ColumnSet::new([
            LibraryColumnId::PlayCount,
            LibraryColumnId::LastPlayed,
            LibraryColumnId::LastLoaded,
            LibraryColumnId::PlayedLast7d,
        ]);
        assert_eq!(
            set.join_fragment().matches("LEFT JOIN").count(),
            1,
            "all four aggregate columns come from one subquery"
        );
    }

    #[test]
    fn analysis_columns_pull_only_the_joins_they_need() {
        let auto_bpm = ColumnSet::new([LibraryColumnId::BpmAuto]);
        assert!(auto_bpm.join_fragment().contains("track_beatgrids xg"));
        assert!(!auto_bpm.join_fragment().contains("track_keys"));

        // LUFS / true peak / prepared all read the analysis cache the
        // fixed SELECT already joins.
        let cached = ColumnSet::new([
            LibraryColumnId::LufsI,
            LibraryColumnId::TruePeak,
            LibraryColumnId::Prepared,
        ]);
        assert_eq!(cached.join_fragment(), "");
    }

    #[test]
    fn joins_are_stable_regardless_of_column_order() {
        let one = ColumnSet::new([LibraryColumnId::InCrates, LibraryColumnId::BpmAuto]);
        let other = ColumnSet::new([LibraryColumnId::BpmAuto, LibraryColumnId::InCrates]);
        assert_eq!(
            one.join_fragment(),
            other.join_fragment(),
            "statement cache hits depend on identical SQL text"
        );
    }

    #[test]
    fn duplicate_ids_collapse() {
        let set = ColumnSet::new([
            LibraryColumnId::Codec,
            LibraryColumnId::Codec,
            LibraryColumnId::FileSize,
        ]);
        assert_eq!(
            set.columns(),
            &[LibraryColumnId::Codec, LibraryColumnId::FileSize]
        );
    }

    #[test]
    fn audio_file_columns_project_their_track_files_columns_once() {
        let set = ColumnSet::new([
            LibraryColumnId::Codec,
            LibraryColumnId::SampleRate,
            LibraryColumnId::Missing,
        ]);
        assert_eq!(
            set.primary_file_columns(),
            vec!["codec", "is_missing", "sample_rate"]
        );
    }

    #[test]
    fn every_column_belongs_to_exactly_one_group_and_has_a_label() {
        for column in LibraryColumnId::all() {
            assert!(
                !column.label().is_empty(),
                "{} needs a label",
                column.as_str()
            );
            assert!(!column.group().label().is_empty());
        }
    }
}
