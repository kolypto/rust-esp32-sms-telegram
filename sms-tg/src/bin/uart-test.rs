#![no_std]
#![no_main]
#![deny(clippy::mem_forget)]

use {esp_backtrace as _, esp_println as _};
esp_bootloader_esp_idf::esp_app_desc!();
use defmt;

use esp_hal::{
    clock::CpuClock,
    interrupt::software::SoftwareInterruptControl,
    timer::timg::TimerGroup,
    uart,
};

use embassy_executor::Spawner;
use embassy_time::Timer;

use sms_tg::linebuf::RingBuffer;

// use static_cell::StaticCell;

#[allow(clippy::large_stack_frames)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // Init Embassy the usual way
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // Init UART
    // TODO: check DMA for faster I/O without CPU
    let mut uart = uart::Uart::new(
        peripherals.UART1,
        uart::Config::default().with_baudrate(115_200),
        ).expect("UART init")
        .with_tx(peripherals.GPIO0)
        .with_rx(peripherals.GPIO1)
        .into_async();
    let (rx, mut tx) = uart.split();

    // Spawn a reader
    spawner.spawn(task_uart_reader(rx)).expect("start uart reader");

    // Write
    let data = b"Hello\n";
    tx.write_async(data).await.expect("UART write");

    loop {
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn task_uart_reader(mut uart: uart::UartRx<'static, esp_hal::Async>) -> ! {
    // Circular buffer
    let mut buf = [0u8; 64];
    let mut rbuf = RingBuffer::new(&mut buf);

    // Complete line
    let mut linebuf = [0u8; 128];
    let mut linebufn = 0;

    loop {
        // Write
        let mut buf = rbuf.writable();
        let n = uart.read_async(&mut buf).await.expect("UART read");
        rbuf.has_written(n);

        // Read all lines
        while let Some(line) = rbuf.read_line(&mut linebuf) && line.len() > 0 {
            // Remove \r
            let line = if line.ends_with(&[b'\r']) { &line[..line.len()-1] } else { line };

            defmt::info!("UART read: {}", line);
            if let Ok(line) = core::str::from_utf8(line) {
                defmt::info!("UART read: {}", line);
            }
        }
    }
}
