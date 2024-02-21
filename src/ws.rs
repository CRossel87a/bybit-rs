use crate::api::{Public, WebsocketAPI};
use crate::client::Client;
use crate::errors::Result;
use crate::model::{Category, PongResponse, Subscription, Tickers, WebsocketEvents};
use crate::util::{build_json_request, generate_random_uid};
use error_chain::bail;
use serde_json::Value;

use std::collections::BTreeMap;
use std::net::TcpStream;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message as WsMessage, WebSocket};

#[derive(Clone)]
pub struct Stream {
    pub client: Client,
}

impl Stream {
    pub fn ws_ping(&self, private: bool) -> Result<()> {
        let mut parameters: BTreeMap<String, Value> = BTreeMap::new();
        parameters.insert("req_id".into(), generate_random_uid(8).into());
        parameters.insert("op".into(), "ping".into());
        let request = build_json_request(&parameters);
        let endpoint = if private {
            WebsocketAPI::Private
        } else {
            WebsocketAPI::Public(Public::Linear)
        };
        let mut response = self.client.wss_connect(
            endpoint,
            Some(request),
            private,
            None,
        )?;
        let data = response.read()?;
        match data {
            WsMessage::Text(data) => {
                let response: PongResponse = serde_json::from_str(&data)?;
                match response {
                    PongResponse::PublicPong(pong) => {
                        println!("Pong received successfully");
                        println!("Connection ID: {}", pong.conn_id);
                    }
                    PongResponse::PrivatePong(pong) => {
                        println!("Pong received successfully");
                        println!("Connection ID: {}", pong.conn_id);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn ws_priv_subscribe<'a, F>(&self, req: Subscription<'a>, handler: F) -> Result<()>
    where
        F: FnMut(WebsocketEvents) -> Result<()> + 'static + Send,
    {
        let request = Self::build_subscription(req);
        let response =
            self.client
                .wss_connect(WebsocketAPI::Private, Some(request), true, Some(9))?;
        Self::event_loop(response, handler)?;
        Ok(())
    }

    pub fn ws_subscribe<'a, F>(
        &self,
        req: Subscription<'a>,
        category: Category,
        handler: F,
    ) -> Result<()>
    where
        F: FnMut(WebsocketEvents) -> Result<()> + 'static + Send,
    {
        let endpoint = {
            match category {
                Category::Linear => WebsocketAPI::Public(Public::Linear),
                Category::Inverse => WebsocketAPI::Public(Public::Inverse),
                Category::Spot => WebsocketAPI::Public(Public::Spot),
                _ => bail!("Option has not been implemented"),
            }
        };
        let request = Self::build_subscription(req);
        let response = self
            .client
            .wss_connect(endpoint, Some(request), false, None)?;
        Self::event_loop(response, handler)?;
        Ok(())
    }

    pub fn build_subscription(action: Subscription) -> String {
        let mut parameters: BTreeMap<String, Value> = BTreeMap::new();
        parameters.insert("req_id".into(), generate_random_uid(8).into());
        parameters.insert("op".into(), action.op.into());
        let args_value: Value = action
            .args
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .into();
        parameters.insert("args".into(), args_value);

        build_json_request(&parameters)
    }

    /// Subscribes to the specified order book updates and handles the order book events
    ///
    /// # Arguments
    ///
    /// * `subs` - A vector of tuples containing the order book ID and symbol
    /// * `category` - The category of the order book
    ///
    /// # Example
    ///
    /// ```
    /// use your_crate_name::Category;
    /// let subs = vec![(1, "BTC"), (2, "ETH")];
    /// ```
    pub fn ws_orderbook(&self, subs: Vec<(i32, &str)>, category: Category) -> Result<()> {
        let arr: Vec<String> = subs
            .into_iter()
            .map(|(num, sym)| format!("orderbook.{}.{}", num, sym.to_uppercase()))
            .collect();
        let request = Subscription::new("subscribe", arr.iter().map(AsRef::as_ref).collect());
        self.ws_subscribe(request, category, |event| {
            if let WebsocketEvents::OrderBookEvent(order_book) = event {
                println!("{:#?}", order_book.data);
            }
            Ok(())
        })
    }

    /// This function subscribes to the specified trades and handles the trade events.
    /// # Arguments
    ///
    /// * `subs` - A vector of trade subscriptions
    /// * `category` - The category of the trades
    ///
    /// # Example
    ///
    /// ```
    /// use your_crate_name::Category;
    /// let subs = vec!["BTCUSD", "ETHUSD"];
    /// let category = Category::Linear;
    /// ws_trades(subs, category);
    /// ```
    pub fn ws_trades(&self, subs: Vec<&str>, category: Category) -> Result<()> {
        let arr: Vec<String> = subs
            .iter()
            .map(|&sub| format!("publicTrade.{}", sub.to_uppercase()))
            .collect();
        let request = Subscription::new("subscribe", arr.iter().map(AsRef::as_ref).collect());
        let handler = |event| {
            if let WebsocketEvents::TradeEvent(trades) = event {
                for trade in trades.data {
                    println!("Trade: {:#?}", trade);
                }
            }
            Ok(())
        };

        self.ws_subscribe(request, category, handler)
    }

    /// Subscribes to ticker events for the specified symbols and category.
    ///
    /// # Arguments
    ///
    /// * `subs` - A vector of symbols for which ticker events are subscribed.
    /// * `category` - The category for which ticker events are subscribed.
    ///
    /// # Examples
    ///
    /// ```
    /// use your_crate_name::Category;
    /// let subs = vec!["BTCUSD", "ETHUSD"];
    /// let category = Category::Linear;
    /// ws_tickers(subs, category);
    /// ```
    pub fn ws_tickers(&self, subs: Vec<&str>, category: Category) -> Result<()> {
        let arr: Vec<String> = subs
            .into_iter()
            .map(|sub| format!("tickers.{}", sub.to_uppercase()))
            .collect();
        let request = Subscription::new("subscribe", arr.iter().map(String::as_str).collect());

        let handler = |event| {
            if let WebsocketEvents::TickerEvent(tickers) = event {
                match tickers.data {
                    Tickers::Linear(linear_ticker) => println!("{:#?}", linear_ticker),
                    Tickers::Spot(spot_ticker) => println!("{:#?}", spot_ticker),
                }
            }
            Ok(())
        };

        self.ws_subscribe(request, category, handler)
    }

    pub fn ws_klines(&self, subs: Vec<(&str, &str)>, category: Category) -> Result<()> {
        let arr: Vec<String> = subs
            .into_iter()
            .map(|(interval, sym)| format!("kline.{}.{}", interval, sym.to_uppercase()))
            .collect();
        let request = Subscription::new("subscribe", arr.iter().map(AsRef::as_ref).collect());
        self.ws_subscribe(request, category, |event| {
            if let WebsocketEvents::KlineEvent(kline) = event {
                for v in kline.data {
                    println!("{:#?}", v);
                }
            }
            Ok(())
        })
    }

    pub fn ws_position(&self, cat: Option<Category>) -> Result<()> {
        let sub_str = if let Some(v) = cat {
            match v {
                Category::Linear => "position.linear",
                Category::Inverse => "position.inverse",
                _ => bail!("Option and Spot has not been implemented"),
            }
        } else {
            "position"
        };

        let request = Subscription::new("subscribe", vec![sub_str]);
        self.ws_priv_subscribe(request, |event| {
            if let WebsocketEvents::PositionEvent(position) = event {
                for v in position.data {
                    println!("{:#?}", v);
                }
            }
            Ok(())
        })
    }

    pub fn ws_executions(&self, cat: Option<Category>) -> Result<()> {
        let sub_str = if let Some(v) = cat {
            match v {
                Category::Linear => "execution.linear",
                Category::Inverse => "execution.inverse",
                Category::Spot => "execution.spot",
                Category::Option => "execution.option",
            }
        } else {
            "execution"
        };

        let request = Subscription::new("subscribe", vec![sub_str]);
        self.ws_priv_subscribe(request, |event| {
            if let WebsocketEvents::ExecutionEvent(execute) = event {
                for v in execute.data {
                    println!("{:#?}", v);
                }
            }
            Ok(())
        })
    }

     pub fn ws_orders(&self, cat: Option<Category>) -> Result<()> {
        let sub_str = if let Some(v) = cat {
            match v {
                Category::Linear => "order.linear",
                Category::Inverse => "order.inverse",
                Category::Spot => "order.spot",
                Category::Option => "order.option",
            }
        } else {
            "order"
        };

        let request = Subscription::new("subscribe", vec![sub_str]);
        self.ws_priv_subscribe(request, |event| {
            if let WebsocketEvents::OrderEvent(order) = event {
                for v in order.data {
                    println!("{:#?}", v);
                }
            }
            Ok(())
        })
    }

    pub fn ws_wallet(&self) -> Result<()> {
        let sub_str = "wallet";
        let request = Subscription::new("subscribe", vec![sub_str]);
        self.ws_priv_subscribe(request, |event| {
            if let WebsocketEvents::Wallet(wallet) = event {
                for v in wallet.data {
                    println!("{:#?}", v);
                }
            }
            Ok(())
        })
    } 

    fn handle_msg(msg: &str, mut parser: impl FnMut(WebsocketEvents) -> Result<()>) -> Result<()> {
        let update: Value = serde_json::from_str(msg)?;

        if let Ok(event) = serde_json::from_value::<WebsocketEvents>(update.clone()) {
            parser(event)?;
        }

        Ok(())
    }

    pub fn event_loop(
        mut stream: WebSocket<MaybeTlsStream<TcpStream>>,
        mut parser: impl FnMut(WebsocketEvents) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        loop {
            let msg = stream.read()?;
            match msg {
                WsMessage::Text(ref msg) => {
                    if let Err(e) = Stream::handle_msg(msg, &mut parser) {
                        bail!(format!("Error on handling stream message: {}", e));
                    }
                }
                _ => {}
            }
        }
    }
}
