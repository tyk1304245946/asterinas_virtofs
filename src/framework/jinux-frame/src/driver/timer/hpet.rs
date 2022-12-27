use acpi::{AcpiError, HpetInfo};
use alloc::vec::Vec;
use volatile::{
    access::{ReadOnly, ReadWrite},
    Volatile,
};

use crate::{
    cell::Cell,
    driver::{
        acpi::ACPI_TABLES,
        ioapic::{self, IoApicEntryHandle},
    },
};
use lazy_static::lazy_static;

lazy_static! {
    static ref HPET_INSTANCE: Cell<HPET> =
        unsafe { Cell::new(core::mem::MaybeUninit::zeroed().assume_init()) };
}

const OFFSET_ID_REGISTER: usize = 0x000;
const OFFSET_CONFIGURATION_REGISTER: usize = 0x010;
const OFFSET_INTERRUPT_STATUS_REGISTER: usize = 0x020;
const OFFSET_MAIN_COUNTER_VALUE_REGISTER: usize = 0x0F0;

const HPET_FREQ: usize = 1_000_000_000_000_000;

#[derive(Debug)]
#[repr(C)]
struct HPETTimerRegister {
    configuration_and_capabilities_register: u32,
    timer_compartor_value_register: u32,
    fsb_interrupt_route_register: u32,
}

struct HPET {
    io_apic_entry: IoApicEntryHandle,
    information_register: Volatile<&'static u32, ReadOnly>,
    general_configuration_register: Volatile<&'static mut u32, ReadWrite>,
    general_interrupt_status_register: Volatile<&'static mut u32, ReadWrite>,

    timer_registers: Vec<Volatile<&'static mut HPETTimerRegister, ReadWrite>>,
}

impl HPET {
    fn new(base_address: usize) -> HPET {
        let information_register_ref = unsafe {
            &*(crate::mm::address::phys_to_virt(base_address + OFFSET_ID_REGISTER) as *mut usize
                as *mut u32)
        };
        let general_configuration_register_ref = unsafe {
            &mut *(crate::mm::address::phys_to_virt(base_address + OFFSET_CONFIGURATION_REGISTER)
                as *mut usize as *mut u32)
        };
        let general_interrupt_status_register_ref = unsafe {
            &mut *(crate::mm::address::phys_to_virt(base_address + OFFSET_INTERRUPT_STATUS_REGISTER)
                as *mut usize as *mut u32)
        };

        let information_register = Volatile::new_read_only(information_register_ref);
        let general_configuration_register = Volatile::new(general_configuration_register_ref);
        let general_interrupt_status_register =
            Volatile::new(general_interrupt_status_register_ref);

        let num_comparator = ((information_register.read() & 0x1F00) >> 8) as u8 + 1;

        let mut comparators = Vec::with_capacity(num_comparator as usize);

        for i in 0..num_comparator {
            let comp = Volatile::new(unsafe {
                &mut *(crate::mm::address::phys_to_virt(base_address + 0x100 + i as usize * 0x20)
                    as *mut usize as *mut HPETTimerRegister)
            });
            comparators.push(comp);
        }

        let mut io_apic_entry = ioapic::IO_APIC.get().allocate_entry().unwrap();
        let vector = super::TIMER_IRQ_NUM;
        // 0 for now
        let destination_apic_id: u8 = 0;
        let write_value = (destination_apic_id as u64) << 56 | vector as u64;

        io_apic_entry.write(write_value);

        HPET {
            io_apic_entry,
            information_register,
            general_configuration_register,
            general_interrupt_status_register,
            timer_registers: comparators,
        }
    }

    pub fn hardware_rev(&self) -> u8 {
        (self.information_register.read() & 0xFF) as u8
    }

    pub fn num_comparators(&self) -> u8 {
        ((self.information_register.read() & 0x1F00) >> 8) as u8 + 1
    }

    pub fn main_counter_is_64bits(&self) -> bool {
        (self.information_register.read() & 0x2000) != 0
    }

    pub fn legacy_irq_capable(&self) -> bool {
        (self.information_register.read() & 0x8000) != 0
    }

    pub fn pci_vendor_id(&self) -> u16 {
        ((self.information_register.read() & 0xFFFF_0000) >> 16) as u16
    }
}

/// HPET init, need to init IOAPIC before init this function
pub fn init() -> Result<(), AcpiError> {
    let c = ACPI_TABLES.lock();

    let hpet_info = HpetInfo::new(&*c)?;

    // config IO APIC entry
    let hpet = HPET::new(hpet_info.base_address);
    *HPET_INSTANCE.get() = hpet;
    Ok(())
}