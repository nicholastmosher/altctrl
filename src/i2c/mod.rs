#![allow(dead_code)]

use std::sync::mpsc::Sender;

use rppal::gpio::{Gpio, InputPin, Level, Trigger};

use i2cdev::core::*;
use i2cdev::linux::{LinuxI2CDevice, LinuxI2CError};

mod constants;
mod port;

use constants::*;
use port::PortStruct;

use crate::protocol::*;
use crate::{Event, SerialEvent};

#[derive(Clone, Debug)]
pub enum I2CEvent {
    On(Device, Port),
    Off(Device, Port),
    Poll(Device),
    ResetState(Device),
}

pub struct I2CStruct {
    pub tx: Sender<Event>,
    pub input_pin_0: InputPin,
    pub i2c_device_0: LinuxI2CDevice,
    pub port_struct_0: [port::PortStruct; 8],
    pub output_struct_0: u8,
}

///The initialize function creates a GPIO (only one per RPi), gets pins for the interrupts,
/// turns the Pins into InputPins, and then configures asynchronos  interrupt functions for the
/// InputPins. There is a function fot the rising edge and the falling edge.
pub fn initialize(tx: Sender<Event>) -> I2CStruct {
    //This is setting up a new GPIO There is only one (1) GPIO for a RPi.
    let gpio = Gpio::new().expect("A new GPIO should have been created");

    //This tries to get a pin from the GPIO. A Pin can be turned into an input pin.
    let maybe_input_pin_0 = gpio
        .get(RPI_GPIO_INT_PIN)
        .expect("A new pin should have been created");
    //Turns Pin into InputPin.
    let mut input_pin_0 = maybe_input_pin_0.into_input_pulldown();

    //Create I2C device
    let mut i2c_device_0 = LinuxI2CDevice::new("/dev/i2c-1", I2C_EXPANDER_0)
        .expect("A new i2c device should have been created");

    //initialize the I2C device
    initialize_i2c_device(&mut i2c_device_0).expect("An i2c device should have been initialied");

    //Initialize the Ports connected to the device
    let port_struct_0 = PortStruct::initialize_device_buttons(Device::D0);

    let tx_clone = tx.clone();

    //Sets the functions for the interrupts on both rising and falling edge
    input_pin_0
        .set_async_interrupt(Trigger::Both, move |level: Level| match level {
            Level::High => tx_clone
                .send(Event::I2C(I2CEvent::Poll(Device::D0)))
                .unwrap(),

            Level::Low => tx_clone
                .send(Event::I2C(I2CEvent::ResetState(Device::D0)))
                .unwrap(),
        })
        .expect("The rising edge interrupt function should be configured");

    I2CStruct {
        tx,
        input_pin_0,
        i2c_device_0,
        port_struct_0,
        output_struct_0: 0,
    }
}

///The handle function assigns functions to events. This one defines the Poll, Reset, On, and Off
/// events for the I2C. This function is called when an I2C event needs to be run.
pub fn handle(message: I2CEvent, i2c_struct: &mut I2CStruct) {
    match message {
        /*
        This will read the interrupt flag pin to determine which button the interrupt came from,
        make sure that the state was changed and send a serial message
        */
        I2CEvent::Poll(device) => {
            let (mut dev, mut port_struct) = {
                match device {
                    Device::D0 => (&mut i2c_struct.i2c_device_0, &mut i2c_struct.port_struct_0),
                }
            };

            let buttons = read_i2c(&mut dev, INTFA).expect("the buttons should have been read");
            read_i2c(&mut dev, INTCAPA).expect("The value of the button should be recorded");

            for x in 0..8 {
                let test = 1 & (buttons >> x) == 1;
                let x_button = port_struct[x];
                if test && !x_button.get_state() {
                    port_struct[x].state = true;

                    i2c_struct
                        .tx
                        .send(Event::Serial(SerialEvent::Pressed(
                            port_struct[x].get_device(),
                            port_struct[x].get_port(),
                        )))
                        .unwrap();
                }
            }
        }

        /*
        This will reset the state, to false, of the button that has been released upon release
        */
        I2CEvent::ResetState(device) => {
            let (mut dev, mut port_struct) = {
                match device {
                    Device::D0 => (&mut i2c_struct.i2c_device_0, &mut i2c_struct.port_struct_0),
                }
            };

            let buttons = read_i2c(&mut dev, INTFA).expect("the buttons should have been read");
            read_i2c(&mut dev, INTCAPA).expect("The value of the button should be recorded");

            for x in 0..8 {
                let test = 1 & (buttons >> x) == 1;
                let x_button = port_struct[x];
                if !test && x_button.get_state() {
                    port_struct[x].state = false;
                    i2c_struct
                        .tx
                        .send(Event::Serial(SerialEvent::Released(
                            port_struct[x].get_device(),
                            port_struct[x].get_port(),
                        )))
                        .unwrap();
                }
            }
        }

        /*
        This will enable the output of the specified port
        */
        I2CEvent::On(device, button) => {
            let (i2c, output) = {
                match device {
                    Device::D0 => (
                        &mut i2c_struct.i2c_device_0,
                        &mut i2c_struct.output_struct_0,
                    ),
                }
            };

            *output |= 1 << button as u8;

            i2c.smbus_write_byte_data(constants::GPIOB, *output)
                .expect("failed to write led enable message");
        }

        /*
        This will disable the output of the specified port
        */
        I2CEvent::Off(device, button) => {
            let (i2c, output) = {
                match device {
                    Device::D0 => (
                        &mut i2c_struct.i2c_device_0,
                        &mut i2c_struct.output_struct_0,
                    ),
                }
            };

            *output &= !(1 << button as u8);

            i2c.smbus_write_byte_data(constants::GPIOB, *output)
                .expect("failed to write led disable message");
        }
    }
}

///Setting up the MCP23017.
fn initialize_i2c_device(dev: &mut LinuxI2CDevice) -> Result<(), LinuxI2CError> {
    //Change bank B to outputs
    dev.smbus_write_byte_data(IODIRB, 0x00)?;
    //Set default values to 0 for bank A and 1 for bank B
    dev.smbus_write_byte_data(DEFVALA, 0x00)?;
    dev.smbus_write_byte_data(DEFVALB, 0xff)?;
    //Set the interrupt logic to compare state to default
    dev.smbus_write_byte_data(INTCONA, 0xff)?;
    //Set open drain
    dev.smbus_write_byte_data(IOCON, 0x02)?;
    //Enable interrupt for all pins on bank A
    dev.smbus_write_byte_data(GPINTAEN, 0xff)?;
    Ok(())
}

///Reads the specified register from the specified device
fn read_i2c(dev: &mut LinuxI2CDevice, register: u8) -> Result<u8, LinuxI2CError> {
    let pin = dev.smbus_read_byte_data(register)?;
    Ok(pin)
}
