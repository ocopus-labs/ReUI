//! Vendor ids that belong to chip makers, not to the product's brand. White-label earbuds and
//! headsets ship the chip vendor's Device ID record unchanged (the "Buds 3r" on the PC this
//! was written on reports Bluetrum 0x0642), so their product ids name a reference design,
//! never a product: the lookup must refuse them and fall through to the name and class.
//!
//! Sources: Bluetooth SIG Assigned Numbers, Company Identifiers (as packaged in Nordic
//! Semiconductor's bluetooth-numbers-database `v1/company_ids.json`); USB-IF vendor ids as
//! listed in the linux-usb `usb.ids` (version 2026.06.26; GPL-2.0-or-later OR BSD-3-Clause).
//! Only the vendor ids are used (facts), no product names.

use super::VidSource;

/// Bluetooth SIG company ids of Bluetooth audio and LE chip makers.
const SIG: &[u16] = &[
    0x0009, // Infineon Technologies AG
    0x000A, // Qualcomm Technologies International (formerly CSR)
    0x000D, // Texas Instruments Inc.
    0x000F, // Broadcom Corporation
    0x001D, // Qualcomm
    0x0046, // MediaTek, Inc.
    0x0059, // Nordic Semiconductor ASA
    0x005D, // Realtek Semiconductor Corporation
    0x0094, // Airoha Technology Corp.
    0x00D7, // Qualcomm Technologies, Inc.
    0x0131, // Cypress Semiconductor
    0x0211, // Telink Semiconductor Co. Ltd
    0x02B0, // Bestechnic (Shanghai), Ltd
    0x02E5, // Espressif Systems (Shanghai) Co., Ltd.
    0x02FF, // Silicon Laboratories
    0x03E0, // Actions Technology Co., Ltd
    0x05D6, // Zhuhai Jieli Technology Co., Ltd
    0x05F0, // Beken
    0x0642, // Bluetrum Technology Co., Ltd
    0x07E3, // Airoha Technology Corp. (second id)
];

/// USB-IF vendor ids of Bluetooth chip makers (LE devices report a USB vendor in their
/// PnP ID characteristic).
const USB: &[u16] = &[
    0x0A12, // Cambridge Silicon Radio, Ltd
    0x0A5C, // Broadcom Corp.
    0x0BDA, // Realtek Semiconductor Corp.
    0x0E8D, // MediaTek Inc.
    0x1915, // Nordic Semiconductor ASA
];

pub fn is_chip_vendor(source: VidSource, vid: u16) -> bool {
    match source {
        VidSource::Bluetooth => SIG.contains(&vid),
        VidSource::Usb => USB.contains(&vid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_catalog_chip_vendors() {
        assert!(is_chip_vendor(VidSource::Bluetooth, 0x0642));
        assert!(is_chip_vendor(VidSource::Bluetooth, 0x0094));
        assert!(!is_chip_vendor(VidSource::Bluetooth, 0x004C)); // Apple
        assert!(!is_chip_vendor(VidSource::Bluetooth, 0x0075)); // Samsung
        assert!(is_chip_vendor(VidSource::Usb, 0x0A12));
        assert!(!is_chip_vendor(VidSource::Usb, 0x045E)); // Microsoft
        // The same number means different companies in the two registries.
        assert!(!is_chip_vendor(VidSource::Usb, 0x0642));
    }
}
