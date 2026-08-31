//! M11f acceptance: export a Dub library and read it back with cues,
//! loops, grids and playlists intact.
//!
//! PRD §8.6 makes this the load-bearing anti-lock-in commitment — "a
//! user who imports a Serato library, builds a Dub crate, and exports
//! to rekordbox XML gets their Serato hot cues back on the other side."
//! The unit tests in `rekordbox_export` prove the serialiser is the
//! reader's inverse; this proves the *database* half, which is where
//! the export actually reaches for a track's path, cues and loops.
//!
//! Deliberately end-to-end through public API only: import a real XML
//! into a real SQLite library, export it, parse the result.

use std::path::{Path, PathBuf};

use dub_library::rekordbox::parse_xml;
use dub_library::rekordbox_export::{collection_to_parsed, to_string};
use dub_library::{import_rekordbox, Library};

fn write_wav(path: &Path) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..4_410 {
        let t = i as f32 / 44_100.0;
        let s = 0.4 * (2.0 * std::f32::consts::PI * 220.0 * t).sin();
        writer
            .write_sample((s * f32::from(i16::MAX)) as i16)
            .unwrap();
    }
    writer.finalize().unwrap();
}

fn location(file: &Path) -> String {
    format!(
        "file://localhost{}",
        file.to_string_lossy().replace(' ', "%20")
    )
}

fn source_xml(files: &[PathBuf]) -> String {
    let mut tracks = String::new();
    for (i, f) in files.iter().enumerate() {
        tracks.push_str(&format!(
            r#"<TRACK TrackID="{id}" Name="Track {id}" Artist="Sound Dimension"
                    Album="Studio One" Genre="Reggae" TotalTime="174"
                    AverageBpm="174.50" Location="{loc}" Tonality="8A">
                 <TEMPO Inizio="0.100" Bpm="174.50" Metro="4/4" Battito="2"/>
                 <POSITION_MARK Name="Intro" Type="0" Start="0.500" Num="-1"/>
                 <POSITION_MARK Name="Drop" Type="0" Start="16.000" Num="1"
                                Red="255" Green="0" Blue="0"/>
                 <POSITION_MARK Name="Roll" Type="4" Start="32.000" End="36.000" Num="0"/>
               </TRACK>"#,
            id = i + 1,
            loc = location(f),
        ));
    }
    let members: String = (0..files.len())
        .map(|i| format!(r#"<TRACK Key="{}"/>"#, i + 1))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.14" Company="AlphaTheta"/>
  <COLLECTION Entries="{n}">{tracks}</COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Type="1" Name="Roots" KeyType="0" Entries="{n}">{members}</NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>"#,
        n = files.len(),
    )
}

/// Import → export → re-import. The cues, loops and grid a DJ brought
/// in from another app must still be there on the way out.
#[test]
fn an_imported_library_exports_with_its_cues_loops_and_grid_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let music = tmp.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let a = music.join("a.wav");
    let b = music.join("b.wav");
    write_wav(&a);
    write_wav(&b);
    let xml = tmp.path().join("rekordbox.xml");
    std::fs::write(&xml, source_xml(&[a.clone(), b.clone()])).unwrap();

    let mut lib = Library::open_at(&tmp.path().join("library.sqlite")).unwrap();
    let summary = import_rekordbox(&mut lib, &xml).expect("import");
    assert_eq!(summary.added, 2, "{summary:?}");

    // The whole point: read it back out.
    let exported = collection_to_parsed(&lib).expect("export");
    let text = to_string(&exported).expect("serialise");
    let back = parse_xml(text.as_bytes()).expect("re-parse our own output");

    assert_eq!(back.tracks.len(), 2, "both tracks survived");

    // Find it by path rather than by position. Export orders by
    // `created_at, id`, and two tracks imported in the same second
    // tie-break on a UUID — stable for a given library, arbitrary
    // across a fresh import. Indexing here would be a flake.
    let t = back
        .tracks
        .iter()
        .find(|t| t.path.as_deref() == Some(a.as_path()))
        .expect("a.wav must be in the export, path intact through the volume/relative-path split");
    assert!(
        back.tracks
            .iter()
            .any(|t| t.path.as_deref() == Some(b.as_path())),
        "b.wav too"
    );

    assert_eq!(t.artist.as_deref(), Some("Sound Dimension"));
    assert_eq!(t.album.as_deref(), Some("Studio One"));
    assert_eq!(t.genre.as_deref(), Some("Reggae"));
    assert_eq!(t.key.as_deref(), Some("8A"), "Tonality round-trips");
    assert!((t.bpm.unwrap() - 174.5).abs() < 1e-6);

    // Grid: anchor, tempo and bar phase.
    assert!((t.grid_anchor_secs.unwrap() - 0.100).abs() < 1e-6);
    assert!((t.grid_bpm.unwrap() - 174.5).abs() < 1e-6);
    assert_eq!(t.grid_bar_phase, 1, "Battito 2 is bar phase 1");

    // Cues: one hot cue on pad 1, one memory cue.
    assert_eq!(t.cues.len(), 2, "cues: {:?}", t.cues);
    let hot = t
        .cues
        .iter()
        .find(|c| c.hotcue == Some(1))
        .expect("the pad-1 hot cue");
    assert!((hot.position_secs - 16.0).abs() < 1e-6);
    assert_eq!(hot.color.as_deref(), Some("#FF0000"));
    assert!(
        t.cues.iter().any(|c| c.hotcue.is_none()),
        "the memory cue must not be promoted to a pad"
    );

    // Loops had no read path in the library at all before M11f.
    assert_eq!(t.loops.len(), 1, "loops: {:?}", t.loops);
    assert!((t.loops[0].start_secs - 32.0).abs() < 1e-6);
    assert!((t.loops[0].end_secs - 36.0).abs() < 1e-6);
}

/// A track with nothing attached still exports as a usable row — this
/// is the common case for a freshly scanned folder.
#[test]
fn a_bare_track_exports_without_a_grid_or_cues() {
    let tmp = tempfile::tempdir().unwrap();
    let music = tmp.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let a = music.join("bare.wav");
    write_wav(&a);
    let xml = tmp.path().join("rekordbox.xml");
    std::fs::write(
        &xml,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <COLLECTION Entries="1">
    <TRACK TrackID="1" Name="Bare" Location="{loc}"/>
  </COLLECTION>
</DJ_PLAYLISTS>"#,
            loc = location(&a)
        ),
    )
    .unwrap();

    let mut lib = Library::open_at(&tmp.path().join("library.sqlite")).unwrap();
    import_rekordbox(&mut lib, &xml).expect("import");

    let exported = collection_to_parsed(&lib).expect("export");
    let back = parse_xml(to_string(&exported).unwrap().as_bytes()).expect("re-parse");

    assert_eq!(back.tracks.len(), 1);
    assert_eq!(back.tracks[0].path.as_deref(), Some(a.as_path()));
    assert!(back.tracks[0].cues.is_empty());
    assert!(back.tracks[0].loops.is_empty());
    assert_eq!(back.tracks[0].grid_bpm, None);
}
