use crate::rita_exit::database::ip_increment::increment;
use crate::rita_exit::database::secs_since_unix_epoch;
use crate::SETTING;
use ::actix_web::Result;
use althea_kernel_interface::ExitClient;
use althea_types::ExitClientIdentity;
use diesel;
use diesel::dsl::{delete, exists};
use diesel::prelude::{ExpressionMethods, PgConnection, QueryDsl, RunQueryDsl};
use diesel::select;
use exit_db::{models, schema};
use failure::Error;
use settings::exit::RitaExitSettings;
use std::net::IpAddr;
use std::net::Ipv4Addr;

/// Takes a list of clients and returns a sorted list of ip addresses spefically v4 since it
/// can implement comparison operators
fn get_internal_ips(clients: &[exit_db::models::Client]) -> Vec<Ipv4Addr> {
    let mut list = Vec::with_capacity(clients.len());
    for client in clients {
        let client_internal_ip = client.internal_ip.parse();
        match client_internal_ip {
            Ok(address) => list.push(address),
            Err(_e) => error!("Bad database entry! {:?}", client),
        }
    }
    // this list should come sorted from the database, this just double checks
    list.sort();
    list
}

/// Gets the next available client ip, takes about O(n) time, we could make it faster by
/// sorting on the database side but I've left that optimization on the vine for now
pub fn get_next_client_ip(conn: &PgConnection) -> Result<IpAddr, Error> {
    use self::schema::clients::dsl::clients;
    let exit_settings = SETTING.get_exit_network();
    let netmask = exit_settings.netmask as u8;
    let start_ip = exit_settings.exit_start_ip;
    let gateway_ip = exit_settings.own_internal_ip;
    // drop here to free up the settings lock, this codepath runs in parallel
    drop(exit_settings);

    let clients_list = clients.load::<models::Client>(conn)?;
    let ips_list = get_internal_ips(&clients_list);
    let mut new_ip: IpAddr = start_ip.into();

    // iterate until we find an open spot, yes converting to string and back is quite awkward
    while ips_list.contains(&new_ip.to_string().parse()?) {
        new_ip = increment(new_ip, netmask)?;
        if new_ip == gateway_ip {
            new_ip = increment(new_ip, netmask)?;
        }
    }
    trace!(
        "The new client's ip is {} selected using {:?}",
        new_ip,
        ips_list
    );

    Ok(new_ip)
}

/// updates the last seen time
pub fn update_client(client: &ExitClientIdentity, conn: &PgConnection) -> Result<(), Error> {
    use self::schema::clients::dsl::{clients, email, last_seen, phone};

    if let Some(mail) = client.reg_details.email.clone() {
        diesel::update(clients.find(&client.global.mesh_ip.to_string()))
            .set(email.eq(mail))
            .execute(&*conn)?;
    }

    if let Some(number) = client.reg_details.phone.clone() {
        diesel::update(clients.find(&client.global.mesh_ip.to_string()))
            .set(phone.eq(number))
            .execute(&*conn)?;
    }

    diesel::update(clients.find(&client.global.mesh_ip.to_string()))
        .set(last_seen.eq(secs_since_unix_epoch() as i64))
        .execute(&*conn)?;

    diesel::update(clients.find(&client.global.mesh_ip.to_string()))
        .set(last_seen.eq(secs_since_unix_epoch() as i64))
        .execute(&*conn)?;

    Ok(())
}

pub fn get_client(ip: IpAddr, conn: &PgConnection) -> Result<models::Client, Error> {
    use self::schema::clients::dsl::{clients, mesh_ip};
    match clients
        .filter(mesh_ip.eq(&ip.to_string()))
        .load::<models::Client>(conn)
    {
        Ok(entry) => Ok(entry[0].clone()),
        Err(e) => {
            error!("We failed to lookup the client {:?} with{:?}", mesh_ip, e);
            bail!("We failed to lookup the client!")
        }
    }
}

/// changes a clients verified value in the database
pub fn verify_client(
    client: &ExitClientIdentity,
    client_verified: bool,
    conn: &PgConnection,
) -> Result<(), Error> {
    use self::schema::clients::dsl::*;

    diesel::update(clients.find(&client.global.mesh_ip.to_string()))
        .set(verified.eq(client_verified))
        .execute(&*conn)?;

    Ok(())
}

/// Marks a client as verified in the database
pub fn verify_db_client(
    client: &models::Client,
    client_verified: bool,
    conn: &PgConnection,
) -> Result<(), Error> {
    use self::schema::clients::dsl::*;

    diesel::update(clients.find(&client.mesh_ip))
        .set(verified.eq(client_verified))
        .execute(&*conn)?;

    Ok(())
}

/// Increments the text message sent count in the database
pub fn text_sent(client: &ExitClientIdentity, conn: &PgConnection, val: i32) -> Result<(), Error> {
    use self::schema::clients::dsl::*;

    diesel::update(clients.find(&client.global.mesh_ip.to_string()))
        .set(text_sent.eq(val + 1))
        .execute(&*conn)?;

    Ok(())
}

pub fn client_exists(ip: &IpAddr, conn: &PgConnection) -> Result<bool, Error> {
    use self::schema::clients::dsl::*;
    trace!("Checking if client exists");
    Ok(select(exists(clients.filter(mesh_ip.eq(ip.to_string())))).get_result(&*conn)?)
}

pub fn delete_client(client: ExitClient, connection: &PgConnection) -> Result<(), Error> {
    use self::schema::clients::dsl::*;
    info!("Deleting clients {:?} in database", client);

    let mesh_ip_string = client.mesh_ip.to_string();
    let statement = clients.find(&mesh_ip_string);
    r#try!(delete(statement).execute(connection));
    Ok(())
}

// for backwards compatibility with entires that do not have a timestamp
// new entires will be initialized and updated as part of the normal flow
pub fn set_client_timestamp(client: ExitClient, connection: &PgConnection) -> Result<(), Error> {
    use self::schema::clients::dsl::*;
    info!("Setting timestamp for client {:?}", client);

    diesel::update(clients.find(&client.mesh_ip.to_string()))
        .set(last_seen.eq(secs_since_unix_epoch()))
        .execute(connection)?;
    Ok(())
}

// we match on email not key? that has interesting implications for
// shared emails
pub fn update_mail_sent_time(
    client: &ExitClientIdentity,
    conn: &PgConnection,
) -> Result<(), Error> {
    use self::schema::clients::dsl::{clients, email, email_sent_time};
    let mail_addr = match client.clone().reg_details.email {
        Some(mail) => mail.clone(),
        None => bail!("Cloud not find email for {:?}", client.clone()),
    };

    diesel::update(clients.filter(email.eq(mail_addr)))
        .set(email_sent_time.eq(secs_since_unix_epoch()))
        .execute(&*conn)?;

    Ok(())
}

pub fn update_low_balance_notification_time(
    client: &ExitClientIdentity,
    conn: &PgConnection,
) -> Result<(), Error> {
    use self::schema::clients::dsl::{clients, last_balance_warning_time, wg_pubkey};

    diesel::update(clients.filter(wg_pubkey.eq(client.global.wg_public_key.to_string())))
        .set(last_balance_warning_time.eq(secs_since_unix_epoch()))
        .execute(&*conn)?;

    Ok(())
}
