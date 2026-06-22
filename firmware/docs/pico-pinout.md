# Raspberry Pi Pico 2 W — pinout reference

Transcribed from the official Pico 2 W pinout diagram (source image:
`.context/attachments/uNeXCD/image.png`). Physical pins 1–40; pin 1 is top-left next to the USB
connector, numbering goes down the left side (1–20), then up the right side (21–40).

This is here to design the **HUB75 pin map** against (done at the HUB75 spike — see
[pico-port.md](pico-port.md)). The annotations below — what the cyw43 radio reserves and which PIO
block it takes — are the part that actually matters for the port.

## Left side (pins 1–20)

| Pin | Name | Alt functions (UART / I2C / SPI) |
|-----|------|----------------------------------|
| 1 | **GP0** | UART0 TX · I2C0 SDA · SPI0 RX |
| 2 | **GP1** | UART0 RX · I2C0 SCL · SPI0 CSn |
| 3 | GND | — |
| 4 | **GP2** | I2C1 SDA · SPI0 SCK |
| 5 | **GP3** | I2C1 SCL · SPI0 TX |
| 6 | **GP4** | UART1 TX · I2C0 SDA · SPI0 RX |
| 7 | **GP5** | UART1 RX · I2C0 SCL · SPI0 CSn |
| 8 | GND | — |
| 9 | **GP6** | I2C1 SDA · SPI0 SCK |
| 10 | **GP7** | I2C1 SCL · SPI0 TX |
| 11 | **GP8** | UART1 TX · I2C0 SDA · SPI1 RX |
| 12 | **GP9** | UART1 RX · I2C0 SCL · SPI1 CSn |
| 13 | GND | — |
| 14 | **GP10** | I2C1 SDA · SPI1 SCK |
| 15 | **GP11** | I2C1 SCL · SPI1 TX |
| 16 | **GP12** | UART0 TX · I2C0 SDA · SPI1 RX |
| 17 | **GP13** | UART0 RX · I2C0 SCL · SPI1 CSn |
| 18 | GND | — |
| 19 | **GP14** | I2C1 SDA · SPI1 SCK |
| 20 | **GP15** | I2C1 SCL · SPI1 TX |

## Right side (pins 21–40)

| Pin | Name | Alt functions / notes |
|-----|------|------------------------|
| 21 | **GP16** | UART0 TX · I2C0 SDA · SPI0 RX |
| 22 | **GP17** | UART0 RX · I2C0 SCL · SPI0 CSn |
| 23 | GND | — |
| 24 | **GP18** | I2C1 SDA · SPI0 SCK |
| 25 | **GP19** | I2C1 SCL · SPI0 TX |
| 26 | **GP20** | I2C0 SDA |
| 27 | **GP21** | I2C0 SCL |
| 28 | GND | — |
| 29 | **GP22** | — |
| 30 | **RUN** | reset (system control) |
| 31 | **GP26** | ADC0 · I2C1 SDA |
| 32 | **GP27** | ADC1 · I2C1 SCL |
| 33 | AGND | analog ground |
| 34 | **GP28** | ADC2 |
| 35 | ADC_VREF | ADC reference |
| 36 | 3V3(OUT) | 3.3 V out (≤300 mA) |
| 37 | 3V3_EN | pull low to disable the 3.3 V regulator |
| 38 | GND | — |
| 39 | **VSYS** | 1.8–5.5 V system input — **feed the 5 V rail here** (see hardware-wiring.md) |
| 40 | **VBUS** | 5 V from USB |

Bottom edge (not in the table above): **SWCLK / GND / SWDIO** — the SWD debug pads. Only needed if
we later add a debug probe; the no-probe flow doesn't use them.

## Reserved / unavailable pins (this is the part that matters)

- **GP23, GP24, GP25, GP29 are NOT on the header** — they wire internally to the **CYW43439**
  radio: `GP23` = power (`WL_ON`), `GP24` = data (`DIO`), `GP25` = chip-select (`CS`), `GP29` =
  clock (`CLK`). embassy drives them as `p.PIN_23/24/25/29`. Don't try to use them for HUB75.
- **The onboard LED is on the CYW43 chip (its GPIO 0), not a pin** — driven via
  `control.gpio_set(0, …)`, only after cyw43 init. (On the non-W Pico it's GP25; not us.)
- **PIO0 is taken by `cyw43-pio`** (it bit-bangs the radio's SPI over PIO). The HUB75 driver must
  use **PIO1** (the RP2350 has PIO0/1/2 — plenty).
- **DMA_CH0 / DMA_CH1** are used by the cyw43 PIO-SPI in the example — give HUB75 its own DMA
  channels.
- **USB** is on dedicated pins (not GPIO), so USB-serial logging costs no header pins.
- **RUN** (pin 30) is reset, not a GPIO.

## Free for HUB75

Everything else: **GP0–GP22, GP26, GP27, GP28** — 26 GPIOs, far more than the 14 HUB75 needs
(6 RGB + 5 address + CLK + LAT + OE). The constraint isn't *count*, it's *arrangement*: the PIO
`out` instruction shifts to **consecutive** pins, so the 6 RGB data lines (and ideally CLK) want a
contiguous block. The actual assignment gets designed at the HUB75 spike with that in mind, then
written into [hardware-wiring.md](hardware-wiring.md) — **don't rewire from the ESP32 GPIO numbers
until then.** Level-shifting (74AHCT245, 3.3 V → 5 V) and panel power are unchanged from the ESP
build; see [hardware-wiring.md](hardware-wiring.md).
