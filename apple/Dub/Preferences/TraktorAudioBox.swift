//
//  TraktorAudioBox.swift
//  Dub
//
//  The Traktor Audio 6 keeps two switches per deck inside the box:
//  Direct Thru (input hardwired to output, the computer muted there) and
//  Phono (turntable or line level on the input). Native Instruments'
//  control panel is the only other way to reach them and is a 32-bit
//  app, which macOS no longer runs — so a DJ on a current Mac is stuck
//  with whatever the box was last left in (Daniel, 2026-11-21).
//
//  They are vendor requests on the device's control pipe, which macOS
//  lets through while its own audio driver has the box. CoreAudio shows
//  neither. The request numbers are the box's protocol as the Linux
//  kernel's USB audio driver documents it; read back off Daniel's box
//  the same day.
//

import Foundation
import IOKit
import IOKit.usb
import IOKit.usb.IOUSBLib

/// One of the box's switches, as the request that reads or sets it.
struct TraktorAudioSwitch: Hashable {
    enum Kind: CaseIterable {
        case directThru, phono
    }

    enum Channel: CaseIterable {
        case a, b

        var letter: String { self == .a ? "A" : "B" }
        /// As printed on the back of the box.
        var pair: String { self == .a ? "3/4" : "5/6" }
    }

    let kind: Kind
    let channel: Channel

    var request: UInt8 { kind == .directThru ? 0x01 : 0x02 }
    /// The first channel of the pair, counted from one.
    var index: UInt16 { channel == .a ? 3 : 5 }
}

/// The connected Traktor Audio 6, for as long as this value lives.
final class TraktorAudioBox {
    static let vendorID: Int32 = 0x17CC
    static let productID: Int32 = 0x1011

    private typealias Device = UnsafeMutablePointer<UnsafeMutablePointer<IOUSBDeviceInterface182>?>
    private let device: Device

    /// Nil when no Traktor Audio 6 is plugged in.
    init?() {
        guard let matching = IOServiceMatching(kIOUSBDeviceClassName) as NSMutableDictionary? else { return nil }
        matching[kUSBVendorID] = Self.vendorID
        matching[kUSBProductID] = Self.productID
        let service = IOServiceGetMatchingService(kIOMainPortDefault, matching)
        guard service != 0 else { return nil }
        defer { IOObjectRelease(service) }

        var plugin: UnsafeMutablePointer<UnsafeMutablePointer<IOCFPlugInInterface>?>?
        var score: Int32 = 0
        guard
            IOCreatePlugInInterfaceForService(
                service, Self.userClientType, Self.pluginInterface, &plugin, &score
            ) == KERN_SUCCESS, let plugin
        else { return nil }
        defer { _ = plugin.pointee?.pointee.Release(plugin) }

        var raw: LPVOID?
        let result = plugin.pointee?.pointee.QueryInterface(
            plugin, CFUUIDGetUUIDBytes(Self.deviceInterface), &raw
        )
        guard result == S_OK, let raw else { return nil }
        device = raw.assumingMemoryBound(to: UnsafeMutablePointer<IOUSBDeviceInterface182>?.self)
    }

    deinit {
        _ = device.pointee?.pointee.Release(device)
    }

    /// Nil when the box did not answer.
    func read(_ which: TraktorAudioSwitch) -> Bool? {
        var value: UInt8 = 0
        let done = withUnsafeMutablePointer(to: &value) { pointer in
            send(which, direction: kUSBIn, value: 0, data: pointer, length: 1)
        }
        return done ? value != 0 : nil
    }

    @discardableResult
    func set(_ which: TraktorAudioSwitch, on: Bool) -> Bool {
        send(which, direction: kUSBOut, value: on ? 1 : 0, data: nil, length: 0)
    }

    private func send(
        _ which: TraktorAudioSwitch, direction: Int, value: UInt16, data: UnsafeMutableRawPointer?, length: UInt16
    ) -> Bool {
        // Bits 7, 6–5, 4–0: direction, type (vendor), recipient (device).
        let type = UInt8(direction << 7) | UInt8(kUSBVendor << 5) | UInt8(kUSBDevice)
        var request = IOUSBDevRequestTO(
            bmRequestType: type, bRequest: which.request, wValue: value, wIndex: which.index,
            wLength: length, pData: data, wLenDone: 0,
            noDataTimeout: Self.timeoutMs, completionTimeout: Self.timeoutMs
        )
        guard let call = device.pointee?.pointee.DeviceRequestTO else { return false }
        return call(device, &request) == kIOReturnSuccess && request.wLenDone == UInt32(length)
    }

    /// The box answers in well under a millisecond; a second is "gone".
    private static let timeoutMs: UInt32 = 1000

    // The header's UUID macros do not reach Swift.
    private static let userClientType = CFUUIDGetConstantUUIDWithBytes(
        nil, 0x9D, 0xC7, 0xB7, 0x80, 0x9E, 0xC0, 0x11, 0xD4, 0xA5, 0x4F, 0x00, 0x0A, 0x27, 0x05, 0x28, 0x61
    )
    private static let pluginInterface = CFUUIDGetConstantUUIDWithBytes(
        nil, 0xC2, 0x44, 0xE8, 0x58, 0x10, 0x9C, 0x11, 0xD4, 0x91, 0xD4, 0x00, 0x50, 0xE4, 0xC6, 0x42, 0x6F
    )
    private static let deviceInterface = CFUUIDGetConstantUUIDWithBytes(
        nil, 0x15, 0x2F, 0xC4, 0x96, 0x48, 0x91, 0x11, 0xD5, 0x9D, 0x52, 0x00, 0x0A, 0x27, 0x80, 0x1E, 0x86
    )
}
