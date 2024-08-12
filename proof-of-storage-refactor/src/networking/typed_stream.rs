use std::io::Error;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio_serde::{formats::Json, Framed};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

// Define the trait `TypedConnection` with associated types for input and output
pub trait TypedConnection<Into, OutOf>
where
    Into: Serialize,
    OutOf: for<'de> Deserialize<'de>,
{
    async fn send(&mut self, data: &Into) -> Result<(), std::io::Error>;
    async fn receive(&mut self) -> Result<OutOf, std::io::Error>;
}


type WrappedStream = FramedRead<OwnedReadHalf, LengthDelimitedCodec>;
type WrappedSink = FramedWrite<OwnedWriteHalf, LengthDelimitedCodec>;

// We use the unit type in place of the message types since we're
// only dealing with one half of the IO
type SerStream<OutOf: for<'de> Deserialize<'de>> = Framed<WrappedStream, OutOf, (), Json<OutOf, ()>>;
type DeSink<Into: Serialize> = Framed<WrappedSink, (), Into, Json<(), Into>>;

pub struct LengthDelimitedTypedConnection<Into, OutOf>
where
    Into: Serialize,
    OutOf: for<'de> Deserialize<'de>,
{
    read_half: SerStream<Into>,
    write_half: DeSink<OutOf>,
}

impl<Into, OutOf> TypedConnection<Into, OutOf> for LengthDelimitedTypedConnection<Into, OutOf>
where
    Into: Serialize,
    OutOf: for<'de> Deserialize<'de>,
{
    async fn send(&mut self, data: &Into) -> Result<(), Error> {
        self.write_half.send(data).await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<OutOf, Error> {
        let received = self.read_half.next().await;
    }
}

#[cfg(test)]
pub mod tests {
    fn test() {
        assert!(true);
    }
}