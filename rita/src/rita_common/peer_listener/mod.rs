/*
PeerListener is used to detect nearby mesh peers, it listens on a ff02::/8 ipv6 address, which is
a link local multicast address, on each listen port. 

On initilization a set of ListenInterface objects are created, these are important becuase they
actually hold the sockets required to listen and broadcast on the listen interfaces, every
rita_loop iteration we send out our own IP as a UDP boradcast packet and then get our peers
off the queue. These are turned into Peer structs which are passed to TunnelManager to do
whatever remaining work there may be. 
*/
use actix::prelude::*;
use actix::{Actor, Context};
use byteorder::{BigEndian, ReadBytesExt};
use bytes::BufMut;
use failure::Error;
use settings::RitaCommonSettings;
use std::collections::HashMap;
use std::io;
use std::io::Cursor;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6, UdpSocket};

use rita_common::rita_loop::Tick;

use KI;
use SETTING;

pub const MSG_IM_HERE: u8 = 0x5b;
pub const MSG_IM_HERE_LEN: u16 = 22;

#[derive(Debug)]
pub struct PeerListener {
    interfaces: HashMap<String, ListenInterface>,
    peers: HashMap<IpAddr, Peer>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Peer {
    pub ifidx: u32,
    pub contact_socket: SocketAddr,
}

impl Peer {
    pub fn new(ip: Ipv6Addr, idx: u32) -> Peer {
        let port = SETTING.get_network().rita_hello_port;
        let socket = SocketAddrV6::new(ip, port.into(), 0, idx);
        Peer {
            ifidx: idx,
            contact_socket: socket.into(),
        }
    }
}

impl Actor for PeerListener {
    type Context = Context<Self>;
}

impl Default for PeerListener {
    fn default() -> PeerListener {
        PeerListener::new().unwrap()
    }
}

impl PeerListener {
    pub fn new() -> Result<PeerListener, Error> {
        Ok(PeerListener {
            interfaces: HashMap::new(),
            peers: HashMap::new(),
        })
    }
}

impl Supervised for PeerListener {}

impl SystemService for PeerListener {
    // Binds to all ready interfaces
    fn service_started(&mut self, _ctx: &mut Context<Self>) {
        info!("PeerListener starting");
        let interfaces = SETTING.get_network().peer_interfaces.clone();
        let iface_list = interfaces;
        for iface in iface_list.iter() {
            let res = ListenInterface::new(iface);
            if res.is_ok() {
                let new_listen_interface = res.unwrap();
                self.interfaces
                    .insert(new_listen_interface.ifname.clone(), new_listen_interface);
            }
        }
    }
}

impl Handler<Tick> for PeerListener {
    type Result = Result<(), Error>;
    fn handle(&mut self, _: Tick, _ctx: &mut Context<Self>) -> Self::Result {
        trace!("Starting PeerListener tick!");
        let res = send_im_here(&mut self.interfaces);
        if res.is_err() {
            error!("Sending ImHere failed with {:?}", res);
        }

        match receive_im_here(&mut self.interfaces) {
            Ok(new_peers) => {
                self.peers = new_peers;
            }
            Err(e) => {
                error!("Receiving ImHere failed with {:?}", e);
            }
        }

        Ok(())
    }
}

// message containing interface name as a string
pub struct Listen(pub String);
impl Message for Listen {
    type Result = ();
}

/// Adds a given interface to the list of interfaces on which peers can be found
/// and contacted
impl Handler<Listen> for PeerListener {
    type Result = ();

    fn handle(&mut self, listen: Listen, _: &mut Context<Self>) -> Self::Result {
        trace!("Peerlistener listen on {:?}", listen.0);
        let new_iface_name = listen.0;

        if self.interfaces.contains_key(&new_iface_name) {
            error!("Someone attempted a double listen!");
            return ();
        }

        let new_iface = ListenInterface::new(&new_iface_name);
        match new_iface {
            Ok(n) => {
                self.interfaces.insert(new_iface_name.clone(), n);
                SETTING
                    .get_network_mut()
                    .peer_interfaces
                    .insert(new_iface_name);
            }
            Err(e) => {
                error!("Peer listener failed to listen on {:?}", e);
            }
        }
    }
}

// message containing interface name as a string
pub struct UnListen(pub String);
impl Message for UnListen {
    type Result = ();
}

/// Removes a given interface to the list of interfaces on which peers can be found
/// and contacted
impl Handler<UnListen> for PeerListener {
    type Result = ();

    fn handle(&mut self, un_listen: UnListen, _: &mut Context<Self>) -> Self::Result {
        trace!("Peerlistener unlisten on {:?}", un_listen.0);
        let ifname_to_delete = un_listen.0;
        if self.interfaces.contains_key(&ifname_to_delete) {
            self.interfaces.remove(&ifname_to_delete);
            SETTING
                .get_network_mut()
                .peer_interfaces
                .remove(&ifname_to_delete);
        } else {
            error!("Tried to unlisten interface that's not present!")
        }
    }
}

#[derive(Debug)]
pub struct GetPeers();
impl Message for GetPeers {
    type Result = Result<HashMap<IpAddr, Peer>, Error>;
}

impl Handler<GetPeers> for PeerListener {
    type Result = Result<HashMap<IpAddr, Peer>, Error>;

    fn handle(&mut self, _: GetPeers, _: &mut Context<Self>) -> Self::Result {
        Ok(self.peers.clone())
    }
}

#[derive(Debug)]
pub struct ListenInterface {
    ifname: String,
    ifidx: u32,
    multicast_socketaddr: SocketAddrV6,
    multicast_socket: UdpSocket,
    linklocal_socket: UdpSocket,
    linklocal_ip: Ipv6Addr,
}

impl ListenInterface {
    pub fn new(ifname: &str) -> Result<ListenInterface, Error> {
        let port = SETTING.get_network().rita_hello_port;
        let disc_ip = SETTING.get_network().discovery_ip;
        debug!("Binding to {:?} for ListenInterface", ifname);
        // Lookup interface link local ip
        let link_ip = KI.get_link_local_device_ip(&ifname)?;

        // Lookup interface index
        let iface_index = match KI.get_iface_index(&ifname) {
            Ok(idx) => idx,
            Err(_) => 0,
        };
        // Bond to multicast discovery address on each listen port
        let multicast_socketaddr = SocketAddrV6::new(disc_ip, port.into(), 0, iface_index);
        let multicast_socket = UdpSocket::bind(multicast_socketaddr)
            .expect("Failed to bind to peer discovery address!");
        let res = multicast_socket.join_multicast_v6(&disc_ip, iface_index);
        trace!("ListenInterface init set multicast v6 with {:?}", res);
        let res = multicast_socket.set_nonblocking(true);
        trace!(
            "ListenInterface multicast init set nonblocking with {:?}",
            res
        );

        let linklocal_socketaddr = SocketAddrV6::new(link_ip, port.into(), 0, iface_index);
        let linklocal_socket = UdpSocket::bind(linklocal_socketaddr).expect(&format!(
            "ListenInterface Failed to bind to link local address {:?} on {:?} with iface_index {:?} ",
            link_ip, ifname, iface_index
        ));
        let res = linklocal_socket.set_nonblocking(true);
        trace!("ListenInterface init set nonblocking with {:?}", res);

        let res = linklocal_socket.join_multicast_v6(&disc_ip, iface_index);
        trace!("ListenInterface Set link local multicast v6 with {:?}", res);

        Ok(ListenInterface {
            ifname: ifname.to_string(),
            ifidx: iface_index,
            multicast_socket: multicast_socket,
            linklocal_socket: linklocal_socket,
            multicast_socketaddr: multicast_socketaddr,
            linklocal_ip: link_ip,
        })
    }
}

/// Creates an ImHere packet with a very simple binary layout
/// magic <u32>, message size <u16>, then IpV6Addr <u128>
fn encode_im_here(addr: Ipv6Addr) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.put_u8(MSG_IM_HERE);
    buf.put_u16_be(MSG_IM_HERE_LEN);
    let ipaddr_bytes: [u8; 16] = addr.octets();
    for i in 0..16 {
        buf.put_u8(ipaddr_bytes[i]);
    }
    trace!("Encoded ImHere packet {:x?}", buf);
    buf
}

#[test]
fn test_encode_im_here() {
    let data = encode_im_here(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xc00a, 0x2ff));
    assert_eq!(
        data,
        vec![
            91, 109, 65, 88, 0, 22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 192, 10, 2, 255,
        ]
    );
}

fn decode_im_here(buf: &Vec<u8>) -> Result<Ipv6Addr, io::Error> {
    trace!("Starting ImHere packet decode!");
    if buf.is_empty() {
        trace!("Recieved an empty ImHere packet!");
        error!("Got an empty packet!");
    }
    let mut pointer = Cursor::new(&buf);
    let packet_magic = pointer.read_u8()?;
    if packet_magic != MSG_IM_HERE {
        trace!(
            "Recieved an ImHere packet with an invalid magic: {:?}",
            packet_magic
        );
        error!("Invalid magic");
    }

    let packet_size = pointer.read_u16::<BigEndian>()?;

    // We check that we can parse the ImHere we expect fully, but allow
    // for trailing data for forwards compatiblity
    if packet_size < MSG_IM_HERE_LEN as u16 {
        trace!(
            "Recieved an ImHere packet with an invalid size: {:?}",
            packet_size
        );
        error!("ImHere is too small");
    }

    let mut peer_address_arr: [u16; 8] = [0xFFFF; 8];
    for i in (0..8).rev() {
        peer_address_arr[i] = pointer.read_u16::<BigEndian>()?;
    }
    let peer_address = Ipv6Addr::new(
        peer_address_arr[7],
        peer_address_arr[6],
        peer_address_arr[5],
        peer_address_arr[4],
        peer_address_arr[3],
        peer_address_arr[2],
        peer_address_arr[1],
        peer_address_arr[0],
    );

    if peer_address.is_unspecified() || peer_address.is_loopback() || peer_address.is_multicast() {
        trace!(
            "Recieved a valid ImHere with an invalid ip address: {:?}",
            peer_address,
        );
        error!("Invalid IP in ImHere");
    }

    trace!("ImHere decoding completed successfully {:?}", peer_address);
    Ok(peer_address)
}

fn send_im_here(interfaces: &mut HashMap<String, ListenInterface>) -> Result<(), Error> {
    trace!("About to send ImHere");
    for obj in interfaces.iter_mut() {
        let listen_interface = obj.1;
        trace!(
            "Sending ImHere to {:?}, with ip {:?}",
            listen_interface.ifname,
            listen_interface.linklocal_ip
        );

        let result = listen_interface.linklocal_socket.send_to(
            &encode_im_here(listen_interface.linklocal_ip.clone()),
            listen_interface.multicast_socketaddr,
        );
        trace!("Sending ImHere to multicast gets {:?}", result);
    }
    Ok(())
}

fn receive_im_here(
    interfaces: &mut HashMap<String, ListenInterface>,
) -> Result<HashMap<IpAddr, Peer>, Error> {
    trace!("About to dequeue ImHere");
    let mut output = HashMap::<IpAddr, Peer>::new();
    for obj in interfaces.iter_mut() {
        let listen_interface = obj.1;
        // Since the only datagrams we are interested in are very small (22 bytes plus overhead)
        // this buffer is kept intentionally small to discard larger packets earlier rather than later
        loop {
            let mut datagram: [u8; 100] = [0; 100];
            let _bytes_read = match listen_interface.multicast_socket.recv_from(&mut datagram) {
                Ok(d) => d,
                Err(e) => {
                    trace!("Out of data on socket wtih message: {:?}", e);
                    break;
                }
            };

            let ipaddr = match decode_im_here(&mut datagram.to_vec()) {
                Ok(ip) => ip,
                Err(e) => {
                    trace!("ImHere decode failed with: {:?}", e);
                    continue;
                }
            };

            if ipaddr == listen_interface.linklocal_ip {
                trace!("Got ImHere from myself");
                continue;
            }

            if output.contains_key(&ipaddr.into()) {
                trace!(
                    "Discarding ImHere We already have a peer with {:?} for this cycle",
                    ipaddr
                );
                continue;
            }
            trace!("ImHere with {:?}", ipaddr);
            let peer = Peer::new(ipaddr, listen_interface.ifidx);
            output.insert(peer.contact_socket.ip(), peer);
        }
    }
    Ok(output)
}
