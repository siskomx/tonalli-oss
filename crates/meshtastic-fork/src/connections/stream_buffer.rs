// Modified for Tonalli: bounded queues, fail-closed overflow, bounded framing and complete writes.
use crate::protobufs;
use log::{debug, error, trace};
use prost::Message;
use thiserror::Error;
use tokio::sync::mpsc::Sender;

use super::wrappers::encoded_data::IncomingStreamData;

/// A struct that represents a buffer of bytes received from a radio stream.
/// This struct is used to store bytes received from a radio stream, and is
/// used to incrementally decode bytes from the received stream into valid
/// FromRadio packets.
#[derive(Clone, Debug)]
pub struct StreamBuffer {
    buffer: Vec<u8>,
    decoded_packet_tx: Sender<protobufs::FromRadio>,
    log_line: Vec<u8>,
}

/// An enum that represents the possible errors that can occur when processing
/// a stream buffer. These errors are used to determine whether the application
/// should wait to receive more data or if the buffer should be purged.
#[derive(Error, Debug, Clone)]
pub enum StreamBufferError {
    #[error("Could not find header sequence [0x94, 0xc3] in buffer")]
    MissingHeaderBytes,
    #[error("Incorrect framing byte: got {found_framing_byte}, expected 0xc3")]
    IncorrectFramingByte { found_framing_byte: u8 },
    #[error("Buffer data is shorter than packet header size: buffer contains {buffer_size} bytes, expected at least {packet_size} bytes")]
    IncompletePacket {
        buffer_size: usize,
        packet_size: usize,
    },
    #[error("Buffer does not contain a value at MSB buffer index of {msb_index}")]
    MissingMSB { msb_index: usize },
    #[error("Buffer does not contain a value at LSB buffer index of {lsb_index}")]
    MissingLSB { lsb_index: usize },
    #[error("Detected malformed packet, packet buffer contains a framing byte at index {next_packet_start_idx}")]
    MalformedPacket { next_packet_start_idx: usize },
    #[error(transparent)]
    DecodeFailure(#[from] prost::DecodeError),
}

const PACKET_HEADER_SIZE: usize = 4;

impl StreamBuffer {
    /// Creates a new StreamBuffer instance that will send decoded FromRadio packets
    /// to the given broadcast channel.
    pub fn new(decoded_packet_tx: Sender<protobufs::FromRadio>) -> Self {
        StreamBuffer {
            buffer: vec![],
            decoded_packet_tx,
            log_line: vec![],
        }
    }

    /// Takes in a portion of a stream message, stores it in a buffer,
    /// and attempts to decode the buffer into valid FromRadio packets.
    ///
    /// # Arguments
    ///
    /// * `message` - A vector of bytes received from a radio stream
    ///
    /// # Example
    ///
    /// ```
    /// let (rx, mut tx) = broadcast::channel::<protobufs::FromRadio>(32);
    /// let buffer = StreamBuffer::new(tx);
    ///
    /// while let Some(message) = stream.try_next().await? {
    ///    buffer.process_incoming_bytes(message);
    /// }
    /// ```
    pub fn process_incoming_bytes(&mut self, message: IncomingStreamData) -> Result<(), ()> {
        if self.buffer.len() + message.as_ref().len() > 66560 { return Err(()); }
        self.buffer.extend_from_slice(message.as_ref());

        // While there are still bytes in the buffer and processing isn't completed,
        // continue processing the buffer
        while !self.buffer.is_empty() {
            let decoded_packet = match self.process_packet_buffer() {
                Ok(packet) => packet,
                Err(err) => match err {
                    StreamBufferError::MissingHeaderBytes => {
                        trace!(
                            "Could not find header sequence [0x94, 0xc3], waiting for more data"
                        );

                        break; // Wait for more data
                    }
                    StreamBufferError::IncorrectFramingByte { found_framing_byte } => {
                        debug!(
                            "Byte {found_framing_byte} not equal to 0xc3, waiting for more data"
                        );

                        break; // Wait for more data
                    }
                    StreamBufferError::IncompletePacket {
                        buffer_size,
                        packet_size,
                    } => {
                        trace!(
                            "Incomplete packet data, expected {packet_size} bytes, found {buffer_size} bytes"
                        );

                        break; // Wait for more data
                    }
                    StreamBufferError::MissingMSB { msb_index } => {
                        trace!("Could not find MSB at index {msb_index}, waiting for more data");

                        break; // Wait for more data
                    }
                    StreamBufferError::MissingLSB { lsb_index } => {
                        trace!("Could not find LSB at index {lsb_index}, waiting for more data");

                        break; // Wait for more data
                    }
                    StreamBufferError::MalformedPacket {
                        next_packet_start_idx,
                    } => {
                        error!(
                              "Detected malformed packet with next packet starting at index {next_packet_start_idx}, purged malformed packet"
                          );

                        continue; // Don't need more data to continue, purge from buffer
                    }
                    StreamBufferError::DecodeFailure { .. } => {
                        error!("Failed to decode chunk from packet, this does not affect the next iteration");

                        continue; // Don't need more data to continue, ignore decode failure
                    }
                },
            };

            trace!("Successfully decoded packet");

            match self.decoded_packet_tx.try_send(decoded_packet) {
                Ok(_) => {
                    trace!("Successfully sent decoded packet");
                    continue;
                }
                Err(e) => {
                    let _ = e;
                    return Err(());
                }
            };
        }

        trace!(
            "Processing complete, buffer contains {} bytes",
            self.buffer.len()
        );
        Ok(())
    }

    /// An internal helper function that is called iteratively on the internal buffer. This
    /// function attempts to decode the buffer into a valid FromRadio packet. This function
    /// will return an error if the buffer does not contain enough data to decode a packet or
    /// if a packet is malformed. This function will return a packet if the buffer contains
    /// enough data to decode a packet, and is able to successfully decode the packet.
    ///
    /// **Note:** This function should only be called when not all received data in the buffer has been processed.
    fn process_packet_buffer(&mut self) -> Result<protobufs::FromRadio, StreamBufferError> {
        trace!(
            "Packet buffer with length {:?}: {:?}",
            self.buffer.len(),
            self.buffer
        );

        // Check that the buffer can potentially contain a packet header
        if self.buffer.len() < PACKET_HEADER_SIZE {
            return Err(StreamBufferError::IncompletePacket {
                buffer_size: self.buffer.len(),
                packet_size: PACKET_HEADER_SIZE,
            });
        }

        self.shift_buffer_to_first_valid_header()?;

        // Note: the framing index is always 0 at this point
        let incoming_packet_data_size = self.get_data_size_from_header()?;

        self.validate_packet_in_buffer(incoming_packet_data_size)?;

        // Get packet data, excluding magic bytes
        let packet_data = self.extract_packet_from_buffer(incoming_packet_data_size)?;

        // Attempt to decode the current packet
        let decoded_packet = protobufs::FromRadio::decode(packet_data.as_slice())?;

        Ok(decoded_packet)
    }

    fn handle_log_bytes(&mut self, idx: usize) {
        let bytes = &self.buffer[0..idx];
        for &b in bytes {
            if b == b'\n' {
                if !self.log_line.is_empty() {
                    let line = String::from_utf8_lossy(&self.log_line);
                    debug!("Radio: {}", line);
                    self.log_line.clear();
                }
            } else if b != b'\r' {
                // ignore carriage return
                if self.log_line.len() >= 1024 { self.log_line.clear(); }
                self.log_line.push(b);
            }
        }
    }

    fn shift_buffer_to_first_valid_header(&mut self) -> Result<(), StreamBufferError> {
        let framing_index = Self::find_framing_index(&self.buffer);

        match framing_index {
            Some(idx) => {
                if idx > 0 {
                    self.handle_log_bytes(idx);
                    self.buffer.drain(0..idx);
                }
                Ok(())
            }
            None => {
                if !self.buffer.is_empty() {
                    let ends_with_start1 = self.buffer.last() == Some(&0x94);
                    let log_len = self.buffer.len() - ends_with_start1 as usize;

                    if log_len > 0 {
                        self.handle_log_bytes(log_len);
                        self.buffer.drain(0..log_len);
                    }
                }
                Err(StreamBufferError::MissingHeaderBytes)
            }
        }
    }

    // All valid packets start with the sequence [0x94 0xc3 size_msb size_lsb], where
    // size_msb and size_lsb collectively give the size of the incoming packet
    // We need to also validate that, if the 0x94 is found and not at the end of the
    // buffer, that the next byte is 0xc3
    // Note that the maximum packet size currently stands at 240 bytes, meaning an MSB is not needed
    fn find_framing_index(buffer: &[u8]) -> Option<usize> {
        // Not possible to have a two-byte sequence in a buffer with less than two bytes
        // Vec::windows will also panic if the buffer is empty
        if buffer.len() < 2 {
            return None;
        }

        buffer.windows(2).position(|b| b == [0x94, 0xc3])
    }

    fn get_data_size_from_header(&mut self) -> Result<usize, StreamBufferError> {
        // Get the "framing byte" after the start of the packet header, or fail if not found
        let found_framing_byte = match self.buffer.get(1) {
            Some(val) => val.to_owned(),
            None => {
                debug!("Could not find framing byte, waiting for more data");
                return Err(StreamBufferError::IncompletePacket {
                    buffer_size: self.buffer.len(),
                    packet_size: PACKET_HEADER_SIZE,
                });
            }
        };

        // Check that the framing byte is correct, and fail if not
        if found_framing_byte != 0xc3 {
            return Err(StreamBufferError::IncorrectFramingByte { found_framing_byte });
        }

        // Get the MSB of the packet header size, or wait to receive all data
        let msb_index: usize = 2;
        let msb = match self.buffer.get(msb_index) {
            Some(val) => val,
            None => {
                return Err(StreamBufferError::MissingMSB { msb_index });
            }
        };

        // Get the LSB of the packet header size, or wait to receive all data
        let lsb_index: usize = 3;
        let lsb = match self.buffer.get(lsb_index) {
            Some(val) => val,
            None => {
                return Err(StreamBufferError::MissingLSB { lsb_index });
            }
        };

        // Combine MSB and LSB of incoming packet size bytes
        // Recall that packet size doesn't include the first four magic bytes
        let incoming_packet_data_size: usize = usize::from(u16::from_le_bytes([*lsb, *msb]));

        Ok(incoming_packet_data_size)
    }

    fn validate_packet_in_buffer(
        &mut self,
        packet_data_size: usize,
    ) -> Result<(), StreamBufferError> {
        if self.buffer.len() < PACKET_HEADER_SIZE + packet_data_size {
            return Err(StreamBufferError::IncompletePacket {
                buffer_size: self.buffer.len(),
                packet_size: PACKET_HEADER_SIZE + packet_data_size,
            });
        }

        let packet_data_start_index = PACKET_HEADER_SIZE;
        let mut packet_data_end_index = packet_data_start_index + packet_data_size;

        // In the event that the last byte is 0x94, we need to account for the possibility of
        // the next byte being 0xc3, which would indicate that the packet is malformed.
        // We can only do this when the buffer has enough data to avoid a slice index panic.
        if self.buffer.len() > packet_data_end_index {
            packet_data_end_index += 1;
        }

        // trace!(
        //     "Validating bytes in range [{}, {})",
        //     packet_data_start_index,
        //     packet_data_start_index + packet_data_size
        // );

        let packet_buffer = self.buffer[packet_data_start_index..packet_data_end_index].to_vec();

        let next_packet_start_index = StreamBuffer::find_framing_index(&packet_buffer)
            // We need to re-normalize to the original buffer since we're working with a sub-slice
            .map(|idx| idx + packet_data_start_index);

        if let Some(next_packet_start_idx) = next_packet_start_index {
            // Remove malformed packet from buffer
            self.buffer.drain(..next_packet_start_idx);

            return Err(StreamBufferError::MalformedPacket {
                next_packet_start_idx,
            });
        }

        Ok(())
    }

    fn extract_packet_from_buffer(
        &mut self,
        packet_data_size: usize,
    ) -> Result<Vec<u8>, StreamBufferError> {
        if self.buffer.len() < packet_data_size {
            return Err(StreamBufferError::IncompletePacket {
                buffer_size: self.buffer.len(),
                packet_size: PACKET_HEADER_SIZE + packet_data_size,
            });
        }

        let packet_start_index = 0;
        let packet_end_index = PACKET_HEADER_SIZE + packet_data_size;

        // Extract packet with header before removing header
        let mut packet_data_with_header: Vec<u8> = self
            .buffer
            .drain(packet_start_index..packet_end_index)
            .collect();

        // trace!(
        //     "Extracted packet data with header of length {:?} from buffer: {:?}",
        //     packet_data_with_header.len(),
        //     packet_data_with_header
        // );

        // Remove header bytes
        let packet_data: Vec<u8> = packet_data_with_header
            .drain(PACKET_HEADER_SIZE..)
            .collect();

        // trace!(
        //     "Extracted packet data of length {:?} from buffer: {:?}",
        //     packet_data.len(),
        //     packet_data
        // );

        Ok(packet_data)
    }
}
