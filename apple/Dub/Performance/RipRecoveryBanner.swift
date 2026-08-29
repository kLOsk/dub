//
//  RipRecoveryBanner.swift
//  Dub
//
//  M26b — the quiet row above the rip bar offering an unfinished rip
//  back to the operator (UI-BACKLOG R-40).
//
//  A rip that died — crash, force-quit, power — leaves its spill and
//  manifest on disk by design. Before this, nothing ever mentioned
//  them again: the side was sitting there, complete, and the only way
//  back to it was the Finder. The banner is deliberately low-key
//  (a row, not a modal): it must not stand between the DJ and the
//  turntable if they came here to do something else.
//
//  Discard leaves the files alone — it only stops the offer for this
//  session. Recorded audio is irreplaceable without setting the needle
//  back down, so nothing here deletes it.
//
//  Pure function of its state, like the rest of the rip surface, so
//  the snapshot suite renders it.
//

import SwiftUI

struct RipRecoveryBannerState: Equatable {
    /// Recorded length in seconds.
    var recordedSecs: Double
    /// The capture died mid-recording rather than being abandoned at
    /// the review screen — worth saying, because it tells the operator
    /// the end of the side may be missing.
    var wasInterrupted: Bool
    /// How many more unfinished rips are waiting behind this one.
    var others: Int = 0

    var durationText: String { RipDuration.text(recordedSecs) }

    var headline: String {
        wasInterrupted
            ? "Unfinished rip — \(durationText) recovered"
            : "Unfinished rip — \(durationText)"
    }
}

struct RipRecoveryBanner: View {

    let state: RipRecoveryBannerState
    var onReview: () -> Void = {}
    var onDismiss: () -> Void = {}

    var body: some View {
        HStack(spacing: DubSpacing.md) {
            Circle()
                .fill(DubColor.stateTentative)
                .frame(width: 8, height: 8)
            Text(state.headline)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
            if state.others > 0 {
                Text("+\(state.others) more")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
            }
            Spacer(minLength: 0)
            Button(action: onReview) {
                Text("Review")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.surface0)
                    .padding(.horizontal, DubSpacing.md)
                    .padding(.vertical, 2)
                    .background(DubColor.textPrimary)
                    .clipShape(Capsule())
            }
            .buttonStyle(.plain)
            .help("Reopen this rip at the split-and-tag screen")
            Button(action: onDismiss) {
                Text("Later")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                    .padding(.horizontal, DubSpacing.md)
                    .padding(.vertical, 2)
                    .background(DubColor.surface2)
                    .clipShape(Capsule())
            }
            .buttonStyle(.plain)
            .help("Keep the recording on disk and stop showing this for now")
        }
        .padding(.horizontal, DubSpacing.md)
        .padding(.vertical, DubSpacing.xs)
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
    }
}

#Preview("Recovery banner — interrupted") {
    RipRecoveryBanner(
        state: RipRecoveryBannerState(recordedSecs: 1_324, wasInterrupted: true))
        .padding()
        .background(DubColor.surface0)
}

#Preview("Recovery banner — abandoned at review") {
    RipRecoveryBanner(
        state: RipRecoveryBannerState(recordedSecs: 812, wasInterrupted: false, others: 2))
        .padding()
        .background(DubColor.surface0)
}
