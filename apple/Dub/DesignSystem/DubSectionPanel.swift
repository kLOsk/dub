//
//  DubSectionPanel.swift
//  Dub
//
//  A titled block of controls: `DubSectionLabel` over its content.
//
//  This is the container the Prep three-column grid is built from, and
//  it is deliberately the *same* container Preferences uses — the two
//  surfaces are both "configure and test" surfaces, so a section should
//  read identically on either. It is promoted out of
//  `PreferencesSheet.section(title:_:)`, which was private to that
//  sheet.
//
//  The panel draws no background or border of its own. Grouping comes
//  from the label and the spacing; boxing every section would add three
//  nested borders to a surface that already sits inside a deck pane
//  inside a window.
//

import SwiftUI

/// A titled control section — label, then `content`, leading-aligned.
struct DubSectionPanel<Content: View>: View {
    let title: String
    /// Passed through to `DubSectionLabel`; `nil` omits the state dot.
    var dot: Color?
    /// Gap between the label and the content beneath it.
    var spacing: CGFloat = DubSpacing.sm
    @ViewBuilder let content: () -> Content

    init(
        _ title: String,
        dot: Color? = nil,
        spacing: CGFloat = DubSpacing.sm,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.dot = dot
        self.spacing = spacing
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: spacing) {
            DubSectionLabel(title, dot: dot)
            content()
        }
    }
}
