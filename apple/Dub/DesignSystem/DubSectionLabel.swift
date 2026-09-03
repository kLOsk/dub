//
//  DubSectionLabel.swift
//  Dub
//
//  The small all-caps label that titles a control section, with the
//  optional state dot some sections carry (siren sounding, rack
//  engaged, key lock active).
//
//  The idiom — `Text(x).font(DubFont.caps).tracking(n)
//  .foregroundStyle(DubColor.textSecondary)`, sometimes preceded by a
//  7 pt `Circle` — was written out by hand at 57 sites across the app
//  with six different tracking values. Two of them (0.6 and 0.8) sat in
//  adjacent rows of the same Prep column.
//

import SwiftUI

/// A section title: `dot` (when the section has a live state to show)
/// then the label in `DubFont.caps`.
struct DubSectionLabel: View {
    let title: String
    /// Colour of the leading state dot. `nil` omits the dot entirely —
    /// a section with no live state should not reserve space for one.
    var dot: Color?
    var tracking: CGFloat = DubFont.capsTracking

    init(_ title: String, dot: Color? = nil, tracking: CGFloat = DubFont.capsTracking) {
        self.title = title
        self.dot = dot
        self.tracking = tracking
    }

    var body: some View {
        HStack(spacing: DubSpacing.xs) {
            if let dot {
                Circle()
                    .fill(dot)
                    .frame(width: 7, height: 7)
            }
            Text(title)
                .font(DubFont.caps)
                .tracking(tracking)
                .foregroundStyle(DubColor.textSecondary)
        }
    }
}
