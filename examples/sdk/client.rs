use rust7::client::{S7Client};
use rust7::client;

fn main() {
    let mut client = S7Client::new();
    let db_number: u16 = 100; // Must exist into the PLC


    match client.connect_s71200_1500("192.168.0.100") {
        Ok(_) => {
            println!("Connected to PLC");
            println!("PDU negotiated: {} byte", client.pdu_length);
            println!("Job time (ms) : {:.3}", client.last_time);
        },
        Err(e) => {
            eprintln!("Connection failed: {}", e);
            return;
        }
    }
    // Reads 462 byte from DB100
    println!("");
    println!("Attempt to read 462 byte from DB100");
    let mut read_buffer = vec![0u8; 462];
    match client.read_db(db_number, 0, &mut read_buffer) {
        Ok(_) => {          
            println!("Success!");
            println!("Job time (ms) : {:.3}", client.last_time);
            println!("Chunks        : {}", client.chunks);           
            println!("Data read:");
            for (i, chunk) in read_buffer.chunks(32).enumerate() {
                print!("{:04X}: ", i * 32); // Hexc Offset
                for byte in chunk {
                    print!("{:02X} ", byte);
                }
                println!();
            }
        },
        Err(e) => eprintln!("Read failed: {}", e),
    }

    // Writes 1024 byte to DB100 
    println!("");
    println!("Attempt to write 1024 byte to DB100");
    let mut write_data = [0u8; 1024];

    for (i, val) in write_data.iter_mut().enumerate() {
        *val = (i % 256) as u8;
    }       
    
    match client.write_db(db_number, 0, &write_data) {
        Ok(_) => { 
            println!("Success!");
            println!("Job time (ms) : {:.3}", client.last_time);
            println!("Chunks        : {}", client.chunks);           
        },
        Err(e) => eprintln!("Write failed: {}", e),
    }

    // Read a bit 
    println!("");
    println!("Attempt to read DB100.DBX45.5");
    match client.read_bit(client::S7_AREA_DB,  db_number, 47, 5) {
        Ok(value) => {
            println!("Success!");
            println!("Job time (ms) : {:.3}", client.last_time);
            println!("Chunks        : {}", client.chunks);           
            println!("Value read    : {}", value)
        },
        Err(e) => eprintln!("Read failed: {}", e),
    }

    // Write a bit 
    println!("");
    println!("Attempt to write 'false' into DB100.DBX16.0");
    match client.write_bit(client::S7_AREA_DB, db_number, 16, 0, false) {
        Ok(_) => {
            println!("Success!");
            println!("Job time (ms) : {:.3}", client.last_time);
            println!("Chunks        : {}", client.chunks);           
        },
        Err(e) => eprintln!("Write failed: {}", e),
    }

    client.disconnect();
    println!("");
    println!("Disconnected");
}