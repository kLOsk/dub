//! Export the library, or one crate, to an interchange file (M11f).

import AppKit
import DubCore
import UniformTypeIdentifiers

/// The save panel behind "Export As…".
///
/// PRD §8.6 is unusually blunt about this one: "Export is a one-click
/// `File → Export Crate As...` UI surface with the target format as a
/// dropdown. Making leaving easy is the load-bearing behaviour: the DJ
/// trusts a tool that doesn't try to trap them. **Bury this and the
/// trust goes.**" So it is a standard `NSSavePanel` with a Format
/// popup, the same shape Preview and Pages use — not a preference, not
/// a submenu three levels down.
enum LibraryExport {

    /// What the popup offers. rekordbox XML first because it is the one
    /// that actually moves a collection; M3U8 is there for the players
    /// that only speak paths.
    enum Format: Int, CaseIterable {
        case rekordboxXml = 0
        case m3u8 = 1

        var menuTitle: String {
            switch self {
            case .rekordboxXml: return "rekordbox XML (grid, cues, loops, key)"
            case .m3u8: return "M3U8 playlist (file paths only)"
            }
        }

        var fileExtension: String {
            switch self {
            case .rekordboxXml: return "xml"
            case .m3u8: return "m3u8"
            }
        }

        var core: LibraryExportFormat {
            switch self {
            case .rekordboxXml: return .rekordboxXml
            case .m3u8: return .m3u8
            }
        }
    }

    /// Present the panel and export. `crateId` of `nil` exports the
    /// whole library.
    ///
    /// Success reveals the file in Finder rather than raising a toast:
    /// the status strip's only channel is an *error* badge, and the
    /// thing a DJ does next with an export is find it to drag into the
    /// other app. `onError` is for failures only; a cancel reports
    /// nothing, because a cancel is not an outcome.
    static func present(
        library: DubLibrary,
        crateId: Int64?,
        crateName: String?,
        onError: @escaping (String) -> Void
    ) {
        let panel = NSSavePanel()
        panel.title = crateName.map { "Export \($0)" } ?? "Export Library"
        panel.prompt = "Export"
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false

        let picker = NSPopUpButton(frame: .zero, pullsDown: false)
        for format in Format.allCases {
            picker.addItem(withTitle: format.menuTitle)
        }
        picker.selectItem(at: 0)

        // The popup has to rename the file as the format changes, or a
        // rekordbox XML gets saved as `.m3u8` and nothing will open it.
        let syncer = ExtensionSyncer(panel: panel, picker: picker)
        picker.target = syncer
        picker.action = #selector(ExtensionSyncer.formatChanged(_:))

        panel.accessoryView = accessoryView(picker: picker)
        panel.nameFieldStringValue = defaultName(crateName: crateName, format: .rekordboxXml)
        panel.allowedContentTypes = [contentType(for: .rekordboxXml)]

        panel.begin { response in
            // Keep the syncer alive until the panel closes; it is only
            // referenced weakly by the popup's target.
            withExtendedLifetime(syncer) {}
            guard response == .OK, let url = panel.url else { return }
            let format = Format(rawValue: picker.indexOfSelectedItem) ?? .rekordboxXml
            do {
                _ = try library.export(
                    outPath: url.path, format: format.core, crateId: crateId)
                NSWorkspace.shared.activateFileViewerSelecting([url])
            } catch {
                onError("Export failed: \(error)")
            }
        }
    }

    private static func defaultName(crateName: String?, format: Format) -> String {
        let base = crateName ?? "Dub Library"
        // Path separators in a crate name would otherwise open the save
        // panel pointing at a directory that does not exist.
        let safe = base.replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: ":", with: "-")
        return "\(safe).\(format.fileExtension)"
    }

    private static func contentType(for format: Format) -> UTType {
        switch format {
        case .rekordboxXml: return .xml
        case .m3u8: return UTType(filenameExtension: "m3u8") ?? .plainText
        }
    }

    private static func accessoryView(picker: NSPopUpButton) -> NSView {
        let label = NSTextField(labelWithString: "Format:")
        label.alignment = .right
        let stack = NSStackView(views: [label, picker])
        stack.orientation = .horizontal
        stack.spacing = 8
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 16, bottom: 12, right: 16)
        return stack
    }

    /// Keeps the save panel's filename extension in step with the
    /// format popup. An `NSObject` because `NSPopUpButton` dispatches
    /// through the ObjC target/action mechanism.
    private final class ExtensionSyncer: NSObject {
        private weak var panel: NSSavePanel?
        private weak var picker: NSPopUpButton?

        init(panel: NSSavePanel, picker: NSPopUpButton) {
            self.panel = panel
            self.picker = picker
        }

        @objc func formatChanged(_ sender: NSPopUpButton) {
            guard let panel, let picker,
                let format = Format(rawValue: picker.indexOfSelectedItem)
            else { return }
            let stem = (panel.nameFieldStringValue as NSString).deletingPathExtension
            panel.allowedContentTypes = [LibraryExport.contentType(for: format)]
            panel.nameFieldStringValue = "\(stem).\(format.fileExtension)"
        }
    }
}
