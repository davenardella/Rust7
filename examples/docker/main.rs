use rust7::client;
use rust7::client::S7Client;

fn read_byte_s7(s7_client: &mut S7Client, db_number: u16) {
    // Reads 16 byte from DB1
    println!("");
    println!("Attempt to read 16 byte from DB1");
    let mut read_buffer = vec![0u8; 16];

    match s7_client.read_db(db_number, 0, &mut read_buffer) {
        Ok(_) => {
            let mut ascii_bytes = [0u8; 16]; // Array of 16 bytes, Since the SoftPLC first 16 Bytes = Hello, S7!

            println!("Success!");
            println!("Job time (ms) : {:.3}", s7_client.last_time);
            println!("Chunks        : {}", s7_client.chunks);
            println!("Data read:");

            for (i, chunk) in read_buffer.chunks(32).enumerate() {
                print!("{:04X}: ", i * 32); // Hex value
                let mut index = 0;

                for (_, plc_byte) in chunk.iter().enumerate() {
                    print!("{:02X} ", plc_byte); // Dec value

                    ascii_bytes[index] = *plc_byte;
                    index += 1;
                }

                println!();
            }

            println!("Dec Array: {:?}", ascii_bytes);
            match std::str::from_utf8(&ascii_bytes) {
                Ok(decoded_string) => println!("{}", decoded_string), // Successfully decode and print = Hello, S7!
                Err(e) => println!("Error decoding ASCII: {}", e),    // Handle error
            }
        }
        Err(e) => eprintln!("Read failed: {}", e),
    }
}

fn read_byte_s7_second_db(s7_client: &mut S7Client, db_number: u16) {
    // Reads 16 byte from DB2
    println!("");
    println!("Attempt to read 16 byte from DB2");
    let mut read_buffer = vec![0u8; 16];

    match s7_client.read_db(db_number, 0, &mut read_buffer) {
        Ok(_) => {
            let mut ascii_bytes = [0u8; 16]; // Array of 16 bytes, Since the SoftPLC first 16 Bytes = Hello, S7!

            println!("Success!");
            println!("Job time (ms) : {:.3}", s7_client.last_time);
            println!("Chunks        : {}", s7_client.chunks);
            println!("Data read:");

            for (i, chunk) in read_buffer.chunks(32).enumerate() {
                print!("{:04X}: ", i * 32); // Hex value
                let mut index = 0;

                for (_, plc_byte) in chunk.iter().enumerate() {
                    print!("{:02X} ", plc_byte); // Dec value

                    ascii_bytes[index] = *plc_byte;
                    index += 1;
                }

                println!();
            }

            println!("Dec Array: {:?}", ascii_bytes);
            match std::str::from_utf8(&ascii_bytes) {
                Ok(decoded_string) => println!("{}", decoded_string), // Successfully decode and print = Hello, S7!
                Err(e) => println!("Error decoding ASCII: {}", e),    // Handle error
            }

            print!("\nDeleting NULL Values");
            const NULL_VALUES: u8 = 0;
            for (i, byte) in ascii_bytes.iter().enumerate() {
                if *byte == NULL_VALUES {
                    print!("\nFound NULL VALUES at index: {:}", i);
                }
            }
            print!("\nLearn to avoid NULL Values");
        }
        Err(e) => eprintln!("Read failed: {}", e),
    }
}

fn read_bit_s7(s7_client: &mut S7Client, db_number: u16) {
    // Read a bit
    println!("\nAttempt to read DB1.DBX72.6"); // 01001000, will be true since it's 1 (Counting <- (7,6,5,4,3,2,1,0))
    match s7_client.read_bit(client::S7_AREA_DB, db_number, 0, 6) {
        Ok(value) => {
            println!("Success!");
            println!("Job time (ms) : {:.3}", s7_client.last_time);
            println!("Chunks        : {}", s7_client.chunks);
            println!("Value read    : {}", value)
        }
        Err(e) => eprintln!("Read failed: {}", e),
    }

    println!("\n[SECOND TIME] Attempt to read DB1.DBX72.7"); // 01001000, will be false since it's 0 (Counting <- (7,6,5,4,3,2,1,0))
    match s7_client.read_bit(client::S7_AREA_DB, db_number, 0, 7) {
        Ok(value) => {
            println!("Success!");
            println!("Job time (ms) : {:.3}", s7_client.last_time);
            println!("Chunks        : {}", s7_client.chunks);
            println!("Value read    : {}", value)
        }
        Err(e) => eprintln!("Read failed: {}", e),
    }

    println!("\n[THIRD TIME] Attempt to read DB1.DBX72.8"); // 01001000, will be throwing an error since it's out of index (Counting <- (7,6,5,4,3,2,1,0))
    match s7_client.read_bit(client::S7_AREA_DB, db_number, 0, 8) {
        Ok(value) => {
            println!("Success!");
            println!("Job time (ms) : {:.3}", s7_client.last_time);
            println!("Chunks        : {}", s7_client.chunks);
            println!("Value read    : {}", value)
        }
        Err(e) => eprintln!("Read failed: {}", e),
    }
}

fn write_byte_s7(s7_client: &mut S7Client, db_number: u16) {
    // Writes 16 byte to DB2, INTENTIONALLY to demonstrate for new people to learn to filter out NULL Values
    println!("");
    println!("Attempt to write 16 byte to DB2");
    let text: &str = "Bye, S7!";
    let ascii_bytes = text.as_bytes(); // Convert string to ASCII bytes
    for byte in ascii_bytes {
        println!("{}", byte); // Print each ASCII byte value
    }

    match s7_client.write_db(db_number, 0, &ascii_bytes) {
        Ok(_) => {
            println!("Success!");
            println!("Job time (ms) : {:.3}", s7_client.last_time);
            println!("Chunks        : {}", s7_client.chunks);
        }
        Err(e) => eprintln!("Write failed: {}", e),
    }
}

fn write_bit_s7(s7_client: &mut S7Client, db_number: u16) {
    // Write a bit
    println!("");
    println!("Attempt to write 'false' into DB1.DBX72.7"); // 01001000, will be true since because index exists (Counting <- (7,6,5,4,3,2,1,0))
    match s7_client.write_bit(client::S7_AREA_DB, db_number, 16, 0, true) {
        Ok(_) => {
            println!("Success!");
            println!("Job time (ms) : {:.3}", s7_client.last_time);
            println!("Chunks        : {}", s7_client.chunks);
        }
        Err(e) => eprintln!("Write failed: {}", e),
    }
}

fn main() {
    let mut client = S7Client::new();
    let db_number: u16 = 1; // Must exist into the PLC
    let db_number_two: u16 = 2; // Must exist into the PLC

    match client.connect_s71200_1500("127.0.0.1") {
        Ok(_) => {
            println!("Connected to PLC");
            println!("PDU negotiated: {} byte", client.pdu_length);
            println!("Job time (ms) : {:.3}", client.last_time);
        }
        Err(e) => {
            eprintln!("Connection failed: {}", e);
            return;
        }
    }

    read_byte_s7(&mut client, db_number);
    write_byte_s7(&mut client, db_number_two);
    read_byte_s7_second_db(&mut client, db_number_two);
    read_bit_s7(&mut client, db_number);
    write_bit_s7(&mut client, db_number);

    client.disconnect();
    println!("");
    println!("Disconnected");
}
