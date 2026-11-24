use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Automatically detects the local IP address used to reach the target server
pub fn get_local_ip_for(target_ip: &str, target_port: u16) -> String {
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect(format!("{}:{}", target_ip, target_port)).is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

/// Checks if a TCP port is open and accepting connections
pub async fn check_tcp_port(server: &str, port: u16, timeout_ms: u64) -> bool {
    let addr = format!("{}:{}", server, port);
    match timeout(Duration::from_millis(timeout_ms), TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => true,
        _ => false,
    }
}

/// Sends a lightweight SIP OPTIONS probe packet to test UDP SIP responsiveness
pub async fn check_udp_sip(server: &str, port: u16, timeout_ms: u64) -> bool {
    let addr = format!("{}:{}", server, port);
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0").await {
        let ping = format!(
            "OPTIONS sip:{} SIP/2.0\r\n\
            Via: SIP/2.0/UDP 127.0.0.1:50600;branch=z9hG4bKprobe\r\n\
            Max-Forwards: 70\r\n\
            To: <sip:{}>\r\n\
            From: <sip:probe@127.0.0.1>;tag=probe1\r\n\
            Call-ID: probe-{}@127.0.0.1\r\n\
            CSeq: 1 OPTIONS\r\n\
            Content-Length: 0\r\n\r\n",
            server, server, uuid::Uuid::new_v4()
        );
        let target: SocketAddr = match addr.parse() {
            Ok(a) => a,
            Err(_) => return false,
        };
        let _ = socket.send_to(ping.as_bytes(), target).await;

        let mut buf = [0u8; 1024];
        match timeout(Duration::from_millis(timeout_ms), socket.recv_from(&mut buf)).await {
            Ok(Ok((len, _))) => len > 0,
            _ => false,
        }
    } else {
        false
    }
}
