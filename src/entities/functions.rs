//! The impls and functions.
//!
//! The /dump-entities endpoint contains a number of independent JSON arrays:
//! 1. keyspaces: "keyspaces": `[{"keyspace_id":"00000000000000000000000000000001","keyspace_name":"system","keyspace_type":"ycql"},..]`
//! 2. tables: "tables": `[{"table_id":"000000010000300080000000000000af","keyspace_id":"00000001000030008000000000000000","table_name":"pg_user_mapping_user_server_index","state":"RUNNING"},..]`
//! 3. tablets: "tablets": `[{"table_id":"sys.catalog.uuid","tablet_id":"00000000000000000000000000000000","state":"RUNNING"},..]`
//!
//!     3.1. replicas: "replicas": `[{"type":"VOTER","server_uuid":"047856aaf11547749694ca7d7941fb31","addr":"yb-2.local:9100"},..]`
//!
//!     This is 3.1 because replicas are arrays nested in tablets.
//!
//! The way these link together is:
//! - `tables.keyspace_id` -> `keyspaces.keyspace_id` (keyspaces.keyspace_id must exist for tables.keyspace_id)
//! - `tables.table_id` -> `tablets.table_id`, which contains the replicas as a nested array.
//!     `tablets.table_id` might not exist for `tables.table_id`, because some tables do not have tablets, such as the postgres catalog entries.
//!
//! # Special keyspaces:
//!
//! | keyspace        | description                                                                                                              |
//! |-----------------|--------------------------------------------------------------------------------------------------------------------------|
//! | system          | contains the local, partitions, roles, transactions, peers, size_estimates, `transactions-<UUID>` tables                 |
//! | system_schema   | contains the YCQL indexes, views, aggregates, keyspaces, tables, types, functions, triggers, columns, sys.catalog tables |
//! | system_auth     | contains the roles, role_permissions, resource_role_permissions_index tables                                             |
//! | template1       | postgres template1 database template, contains catalog                                                                   |
//! | template0       | postgres template0 database template, contains catalog                                                                   |
//! | postgres        | postgres standard database, not commonly used with YugabyteDB                                                            |
//! | yugabyte        | postgres standard database, default database                                                                             |
//! | system_platform | postgres database, contains a few extra catalog tables starting with 'sql'                                               |
//!
//! # YCQL
//! YCQL requires a keyspace to be defined before user objects can be created and loaded, and
//! keyspace, table and tablet will get a random UUID as id.
//!
//! # YSQL
//! YSQL keyspaces do get an id in a specific way.
//! The id of the YSQL keyspace is in a format that later is used by the objects too.
//! This is how a YSQL keyspace id looks like:
//!
//! ```text
//! | 0  1  2  3| 4  6| 7| 8| 9|10 11|12 13 14 15|
//! |-----------|-----|--|--|--|-----|-----------|
//! |00 00 33 e5|00 00|30|00|80|00 00|00 00 00 00|
//! db                ver   var      object
//! oid               (3)   (8)      oid
//! (hex)                   (hex)
//! ```
//! - (position 12..15): A YSQL keyspace has the object id set to all 0. This is described in the YugabyteDB sourcecode: src/yb/common/entity_ids.cc
//! - (position 7): Version 3 is for ISO4122 UUID version.
//! - (position 9): Variant 8 means DCE 1.1, ISO/IEC 11578:1996
//! - Version and variant are static currently.
//!
//! ## YSQL object id and catalog data
//! The object OID number indicates whether an object is a catalog object or a user object.
//! Any object OID lower than 16384 (0x4000) is a catalog object. This is why a user table_id always starts from ..4000.
//!
//! ## YSQL colocated databases.
//! If a database is created with colocation turned on, it will generate a special table entry:
//! ```text
//!     {
//!       "table_id": "0000400f000030008000000000000000.colocated.parent.uuid",
//!       "keyspace_id": "0000400f000030008000000000000000",
//!       "table_name": "0000400f000030008000000000000000.colocated.parent.tablename",
//!       "state": "RUNNING"
//!     }
//! ```
//! This indicates the keyspace/database is colocated, and thus any object not explicitly defined using its own tablets,
//! will be stored in the tablets that are part of the database.
//! 
use chrono::Local;
use std::{time::Instant, sync::mpsc::channel};
use log::*;
use regex::Regex;
use colored::*;
use anyhow::{Result, bail};
use crate::isleader::AllIsLeader;
use crate::utility;
use crate::snapshot;
use crate::entities::{Entities, AllEntities, EntitiesDiff, KeyspaceDiff, TablesDiff, TabletsDiff, ReplicasDiff};
use crate::health_check::AllHealthCheck;
use crate::Opts;

impl Entities {
    fn new() -> Self {
        Default::default()
    }
}

impl AllEntities
{
    pub async fn perform_snapshot(
        hosts: &Vec<&str>,
        ports: &Vec<&str>,
        snapshot_number: i32,
        parallel: usize,
    ) -> Result<()>
    {
        info!("begin snapshot");
        let timer = Instant::now();

        let allentities = AllEntities::read_entities(hosts, ports, parallel).await;
        snapshot::save_snapshot_json(snapshot_number,"entities", allentities.entities)?;

        info!("end snapshot: {:?}", timer.elapsed());

        Ok(())
    }
    pub fn new() -> Self {
        Default::default()
    }
    pub async fn read_entities (
        hosts: &Vec<&str>,
        ports: &Vec<&str>,
        parallel: usize,
    ) -> AllEntities
    {
        info!("begin parallel http read");
        let timer = Instant::now();
        //let (tx, rx) = channel();

        /*
        let mut host = String::new();
        let mut port = String::new();
        for ref_host in hosts {
            for ref_port in ports {
                let tx = tx.clone();
                host = ref_host.to_string();
                port = ref_port.to_string();
                let handle = tokio::task::spawn_blocking(|| async move {
                    info!("before time");
                    let detail_snapshot_time = Local::now();
                    debug!("before read http");
                    let entities = AllStoredEntities::read_http(host.clone(), port.clone()).await;
                    //tx.send((format!("{}:{}", host, port), detail_snapshot_time, entities)).await.expect("error sending data via tx (entities)");
                    debug!("before send");
                    tx.send((format!("{}:{}", host, port), detail_snapshot_time, entities)).unwrap();
                });
                handles.push(handle);
                /*
                s.spawn(move |_| {
                    let detail_snapshot_time = Local::now();
                    let entities = AllStoredEntities::read_http(host, port);
                    tx.send((format!("{}:{}", host, port), detail_snapshot_time, entities)).expect("error sending data via tx (entities)");
                });
                 */
            }
        }

         */

        let pool = rayon::ThreadPoolBuilder::new().num_threads(parallel).build().unwrap();
        let (tx, rx) = channel();
        pool.scope(move |s| {
            for host in hosts {
                for port in ports {
                    let tx = tx.clone();
                    s.spawn(move |_|  {
                        let detail_snapshot_time = Local::now();
                        let mut entities = AllEntities::read_http(host, port);
                        entities.timestamp = Some(detail_snapshot_time);
                        entities.hostname_port = Some(format!("{}:{}", host, port));
                        tx.send(entities).expect("error sending data via tx");
                    });
                }
            }
        });

        info!("end parallel http read {:?}", timer.elapsed());

        let mut allentities = AllEntities::new();

        for entity in rx.iter().filter(|r| !r.keyspaces.is_empty() && !r.tables.is_empty() && !r.tablets.is_empty()) {
            allentities.entities.push(entity);
        }

        allentities
    }
    fn read_http(
        host: &str,
        port: &str,
    ) -> Entities
    {
        let data_from_http = utility::http_get(host, port, "dump-entities");
        AllEntities::parse_entities(data_from_http, host, port)
    }
    fn parse_entities(
        entities_data: String,
        host: &str,
        port: &str,
    ) -> Entities
    {
        serde_json::from_str(&entities_data )
            .unwrap_or_else(|e| {
                debug!("({}:{}) could not parse /dump-entities json data for entities, error: {}", host, port, e);
                Entities::new()
            })
    }
    pub fn print_coloc_leader_host(
        &self,
        leader_hostname: String,
        colocated_database: &str,
    ) -> Result<()>
    {
        for entity in self.entities.iter()
        {
            // select the information from the master leader
            if entity.hostname_port.ne(&Some(leader_hostname.clone()))
            {
                continue;
            }
            // investigate keyspaces
            for row in &entity.keyspaces
            {
                if row.keyspace_name != *colocated_database
                {
                    debug!("Found keyspace: {}, is not {}", row.keyspace_name, colocated_database.to_owned());
                    continue;
                }
                // a ysql keyspace is not removed from keyspaces when it's dropped.
                // to identify a dropped keyspace, we can count the number of tables that reside in it.
                // if the number is 0, it is a dropped keyspace.
                if row.keyspace_type == "ysql"
                    && entity.tables.iter()
                    .filter(|r| r.keyspace_id == row.keyspace_id)
                    .count() > 0
                {
                    // a ysql keyspace is a colocated keyspace if it a tablet exists
                    // with the following table_id: <keyspace_id>.colocated.parent.uuid
                    if entity.tablets.iter()
                        .any(|r| r.table_id == format!("{}.colocated.parent.uuid", &row.keyspace_id))
                    {
                        // We got a colocated YSQL database!
                        if let Some(tablet) = entity.tablets
                            .iter()
                            .find(|r| r.table_id == format!("{}.colocated.parent.uuid", &row.keyspace_id))
                        {
                            let leader_host = tablet.replicas
                                .clone()
                                .unwrap_or_default()
                                .iter()
                                .map(|r|
                                    {
                                        if &r.server_uuid == tablet.leader.as_ref().unwrap_or(&"".to_string())
                                        {
                                            (&r.addr.split(':').next().unwrap_or_default()).to_string()
                                        }
                                        else
                                        {
                                            "".to_string()
                                        }
                                    })
                                .collect::<String>();
                            if !leader_host.is_empty()
                            {
                                println!("{}", leader_host);
                                return Ok(())
                            }
                            else
                            {
                                bail!("No tablet leader host found.");
                            };
                        }
                    }
                    else
                    {
                       bail!("Database {} is not colocated!", colocated_database.to_owned());
                    };
                }
                else
                {
                    // Explicitly check for more than zero tables: continue if zero tables are found,
                    // so dropped keyspaces are skipped, so we can find another one with the same name.
                    // this allows to find a keyspace by name that previously was created with the same
                    // name and was dropped.
                    if entity.tables.iter()
                        .filter(|r| r.keyspace_id == row.keyspace_id)
                        .count() != 0
                    {
                        bail!("Database found, but not suitable for unknown reason. This should not happen.")
                    }
                }
            }
        }
        bail!("Database name not found.")
    }
    pub fn print(
        &self,
        table_name_filter: &Regex,
        details_enable: &bool,
        leader_hostname: String,
        hostname_filter: &Regex,
        dead_nodes: Vec<String>,
        under_replicated_tablets: Vec<String>,
    ) -> Result<()>
    {
        let is_system_keyspace = |keyspace: &str| -> bool {
            matches!(keyspace, "00000000000000000000000000000001" |   // ycql system
                               "00000000000000000000000000000002" |   // ycql system_schema
                               "00000000000000000000000000000003" |   // ycql system_auth
                               "00000001000030008000000000000000" |   // ysql template1
                               "000033e5000030008000000000000000")    // ysql template0
        };

        for entity in self.entities.iter()
        {
            // only pick the leader hostname if details_enable is not set
            if !*details_enable && entity.hostname_port.ne(&Some(leader_hostname.clone()))
            {
                continue;
            }
            // if details_enable is set, apply hostname_filter.
            if *details_enable && !hostname_filter.is_match(entity.hostname_port.clone().unwrap().as_ref())
            {
                continue;
            }
            for row in &entity.keyspaces
            {
                // do not show "system" keyspaces with normal (non details-enable) usage.
                if !*details_enable
                    && is_system_keyspace(row.keyspace_id.as_str())
                {
                    continue;
                }
                // a ysql keyspace is not removed from keyspaces when it's dropped.
                // to identify a dropped keyspace, we can count the number of tables that reside in it.
                // if the number is 0, it is a dropped keyspace.
                if row.keyspace_type == "ysql"
                    && entity.tables.iter()
                        .filter(|r| r.keyspace_id == row.keyspace_id)
                        .count() > 0
                {
                    // a ysql keyspace is a colocated keyspace if it a tablet exists
                    // with the following table_id: <keyspace_id>.colocated.parent.uuid
                    // the variable "colocation" is set to "[colocated]" if that is true for the current keyspace.
                    let colocation = if entity.tablets.iter()
                        .any(|r| r.table_id == format!("{}.colocated.parent.uuid", &row.keyspace_id))
                    {
                        "[colocated]"
                    } else {
                        ""
                    };
                    // print keyspace details.
                    if *details_enable
                    {
                        print!("{} ", entity.hostname_port.clone().unwrap());
                    }
                    println!("Keyspace:     {}.{} id: {} {}", row.keyspace_type, row.keyspace_name, row.keyspace_id, colocation);
                    // if the keyspace is colocated, it means it has got a tablet directly linked to it.
                    // normally a tablet is linked to a table.
                    // if colocated is true, print the tablet and replica details:
                    if colocation == "[colocated]"
                    {
                        for tablet in entity.tablets
                            .iter()
                            .filter(|r| r.table_id == format!("{}.colocated.parent.uuid", &row.keyspace_id))
                        {
                            if *details_enable
                            {
                                print!("{} ", entity.hostname_port.clone().unwrap());
                            }
                            let under_replication_warning = if under_replicated_tablets
                                .iter()
                                .any(|r| *r == tablet.tablet_id)
                            {
                                "[UNDER REPLICATED]".yellow()
                            }
                            else
                            {
                                "".yellow()
                            };
                            println!("  Tablet:     {}.{}.{} state: {} {}",
                                     row.keyspace_type,
                                     row.keyspace_name,
                                     tablet.tablet_id,
                                     tablet.state,
                                     under_replication_warning,
                            );
                            // replicas
                            if *details_enable
                            {
                                print!("{} ", entity.hostname_port.clone().unwrap());
                            }
                            println!("    Replicas: ({})", tablet.replicas.clone()
                                .unwrap_or_default()
                                .iter()
                                .map(|r|
                                    {
                                        if &r.server_uuid == tablet.leader.as_ref().unwrap_or(&"".to_string())
                                        {
                                            let server_liveness_indicator = if dead_nodes
                                                .iter()
                                                .any(|dead_server| dead_server == &r.server_uuid)
                                            {
                                                "[DEAD]".red()
                                            }
                                            else
                                            {
                                                "".red()
                                            };
                                            format!("{}({}:LEADER{}), ", &r.addr, &r.replica_type, server_liveness_indicator)
                                        }
                                        else
                                        {
                                            let server_liveness_indicator = if dead_nodes
                                                .iter()
                                                .any(|dead_server| dead_server == &r.server_uuid)
                                            {
                                                "[DEAD]".red()
                                            }
                                            else
                                            {
                                                "".red()
                                            };
                                            format!("{}({}{}), ", &r.addr, &r.replica_type, server_liveness_indicator)
                                        }
                                    })
                                .collect::<String>()
                                .trim()
                            );
                        }
                    }
                } else if row.keyspace_type != "ysql"
                {
                    if *details_enable
                    {
                        print!("{} ", entity.hostname_port.clone().unwrap());
                    }
                    println!("Keyspace:     {}.{} id: {}", row.keyspace_type, row.keyspace_name, row.keyspace_id);
                }
            }
            let object_oid_number = |oid: &str| -> u32 {
                // The oid entry normally is a 32 byte UUID for both ycql and ysql, which only contains hexadecimal numbers.
                // However, there is a single entry in system_schema that has a table_id that is 'sys.catalog.uuid'
                if oid.len() == 32_usize {
                    let true_oid = &oid[24..];
                    u32::from_str_radix(true_oid, 16).unwrap()
                } else {
                    0
                }
            };
            for row in entity.tables.iter() {
                if is_system_keyspace(row.keyspace_id.as_str()) && !*details_enable
                {
                    continue
                }
                if !table_name_filter.is_match(&row.table_name)
                {
                    continue
                }
                // ysql table_id has got the OID number in it,
                // the below function takes that, and tests if it's below 16384.
                // ysql oid numbers below 16384 are system/catalog tables.
                //
                // The purpose is to skip ysql catalog tables when details_enable is not set.
                if object_oid_number(row.table_id.as_str()) < 16384
                    && !*details_enable
                    // This takes the keyspace_id from the tables struct row,
                    // and filters the contents of keyspaces vector for the keyspace_id.
                    // Using the keyspace_id, it maps the keyspace_name,
                    // which is tested for equality with "ysql" to make sure it's a ysql row for which the filter is applied.
                    && entity.keyspaces.iter()
                        .find(|r| r.keyspace_id == row.keyspace_id)
                        .map(|r| &r.keyspace_type)
                        .unwrap() == "ysql"
                {
                    continue
                }
                // if the table is colocated, it means it does not have one or more tablets
                // directly linked to the table.
                // To check for colocation:
                // - the table keyspace type must be ysql.
                // - the must be OID >= 16384 (catalog tables do not have tablets).
                // - there are no tablets directly linked to the table.
                let colocation = if entity.keyspaces
                        .iter()
                        .find(|r| r.keyspace_id == row.keyspace_id)
                        .map(|r| r.keyspace_type.clone())
                        .unwrap_or_default() == "ysql"
                    && object_oid_number(row.table_id.as_str()) >= 16384
                    && !entity.tablets
                        .iter()
                        .any(|r| r.table_id == row.table_id)
                {
                    "[colocated]"
                }
                else
                {
                    ""
                };
                if *details_enable
                {
                    print!("{} ", entity.hostname_port.clone().unwrap());
                }
                println!("Object:       {}.{}.{}, state: {}, id: {} {}",
                    &entity.keyspaces
                        .iter()
                        .find(|r| r.keyspace_id == row.keyspace_id)
                        .map(|r| r.keyspace_type.clone())
                        .unwrap_or_default(),
                    &entity.keyspaces
                        .iter()
                        .find(|r| r.keyspace_id == row.keyspace_id)
                        .map(|r| r.keyspace_name.clone())
                        .unwrap_or_default(),
                    row.table_name,
                    row.state,
                    row.table_id,
                    colocation,
                );
                for tablet in entity.tablets
                    .iter()
                    .filter(|r| r.table_id == row.table_id)
                {
                    if *details_enable
                    {
                        print!("{} ", entity.hostname_port.clone().unwrap());
                    }
                    let under_replication_warning = if under_replicated_tablets
                        .iter()
                        .any(|r| *r == tablet.tablet_id)
                    {
                        "[UNDER REPLICATED]".yellow()
                    }
                    else
                    {
                        "".yellow()
                    };
                    println!("  Tablet:     {}.{}.{}.{} state: {} {}",
                             &entity.keyspaces
                                 .iter()
                                 .find(|r| r.keyspace_id == row.keyspace_id)
                                 .map(|r| r.keyspace_type.clone())
                                 .unwrap_or_default(),
                             &entity.keyspaces
                                 .iter()
                                 .find(|r| r.keyspace_id == row.keyspace_id)
                                 .map(|r| r.keyspace_name.clone())
                                 .unwrap_or_default(),
                             row.table_name,
                             tablet.tablet_id,
                             tablet.state,
                             under_replication_warning,
                    );
                    // replicas
                    if *details_enable
                    {
                        print!("{} ", entity.hostname_port.clone().unwrap());
                    }
                    println!("    Replicas: ({})", tablet.replicas.clone()
                        .unwrap_or_default()
                        .iter()
                        .map(|r|
                            {
                                if &r.server_uuid == tablet.leader.as_ref().unwrap_or(&"".to_string())
                                {
                                    let server_liveness_indicator = if dead_nodes
                                        .iter()
                                        .any(|dead_server| dead_server == &r.server_uuid)
                                    {
                                        "[DEAD]".red()
                                    }
                                    else
                                    {
                                        "".red()
                                    };
                                    format!("{}({}:LEADER{}), ", &r.addr, &r.replica_type, server_liveness_indicator)
                                }
                                else
                                {
                                    let server_liveness_indicator = if dead_nodes
                                        .iter()
                                        .any(|dead_server| dead_server == &r.server_uuid)
                                    {
                                        "[DEAD]".red()
                                    }
                                    else
                                    {
                                        "".red()
                                    };
                                    format!("{}({}{}), ", &r.addr, &r.replica_type, server_liveness_indicator)
                                }
                            })
                        .collect::<String>()
                        .trim()
                    );
                }
            }
        };
        Ok(())
    }
}

impl EntitiesDiff {
    pub fn new() -> Self { Default::default() }
    pub fn snapshot_diff(
        begin_snapshot: &String,
        end_snapshot: &String,
    ) -> Result<EntitiesDiff>
    {
        let mut entitiesdiff = EntitiesDiff::new();

        let mut allentities = AllEntities::new();
        allentities.entities = snapshot::read_snapshot_json(begin_snapshot, "entities")?;
        let master_leader = AllIsLeader::return_leader_snapshot(begin_snapshot)?;
        entitiesdiff.first_snapshot(allentities, master_leader);

        let mut allentities = AllEntities::new();
        allentities.entities = snapshot::read_snapshot_json(end_snapshot, "entities")?;
        let master_leader = AllIsLeader::return_leader_snapshot(begin_snapshot)?;
        entitiesdiff.second_snapshot(allentities, master_leader);

        Ok(entitiesdiff)
    }
    fn first_snapshot(
        &mut self,
        allentities: AllEntities,
        master_leader: String,
    )
    {
        if master_leader == *"" {
            self.master_found = false;
            return
        } else {
            self.master_found = true;
        }
        trace!("First snapshot: master_leader: {}, found: {}", master_leader, self.master_found);

        for entity in allentities.entities
            .iter()
            .filter(|r| r.hostname_port.as_ref().unwrap().clone() == master_leader.clone())
        {
            for keyspace in &entity.keyspaces {
                self.btreekeyspacediff
                    .entry(keyspace.keyspace_id.clone())
                    .and_modify(|_| error!("Duplicate keyspace_id entry: id: {}, type: {}, name: {}",
                        keyspace.keyspace_id,
                        keyspace.keyspace_type,
                        keyspace.keyspace_name)
                    )
                    .or_insert(KeyspaceDiff {
                        first_keyspace_name: keyspace.keyspace_name.clone(),
                        first_keyspace_type: keyspace.keyspace_type.clone(),
                        ..Default::default()
                        }
                    );
            }
            for table in &entity.tables {
                self.btreetablesdiff
                    .entry(table.table_id.clone())
                    .and_modify(|_| error!("Duplicate table_id entry: id: {}, name: {}, keyspace_id: {}, state: {}",
                        table.table_id,
                        table.keyspace_id,
                        table.table_name,
                        table.state)
                    )
                    .or_insert( TablesDiff {
                        first_keyspace_id: table.keyspace_id.clone(),
                        first_table_name: table.table_name.clone(),
                        first_state: table.state.clone(),
                        ..Default::default()
                    });
            }
            for tablet in &entity.tablets {
                self.btreetabletsdiff
                    .entry(tablet.tablet_id.clone())
                    .and_modify(|_| error!("Duplicate tablet_id entry: id: {}, table_id: {}, state: {}, leader: {}",
                        tablet.tablet_id,
                        tablet.table_id,
                        tablet.state,
                        tablet.leader.as_ref().unwrap_or(&"".to_string()))
                    )
                    .or_insert( TabletsDiff {
                        first_table_id: tablet.table_id.clone(),
                        first_state: tablet.state.clone(),
                        first_leader: tablet.leader.as_ref().unwrap_or(&"".to_string()).to_string(),
                        ..Default::default()
                    });
                for replica in tablet.replicas.clone().unwrap_or_default() {
                    self.btreereplicasdiff
                        .entry( (tablet.tablet_id.clone(), replica.server_uuid.clone()) )
                        .and_modify(|_| error!("Duplicate replica entry: ({}, {}), addr: {}, type: {}",
                            tablet.tablet_id,
                            replica.server_uuid,
                            replica.addr,
                            replica.replica_type)
                        )
                        .or_insert( ReplicasDiff {
                            first_addr: replica.addr,
                            first_replica_type: replica.replica_type,
                            ..Default::default()
                        });
                }
            }
        }
    }
    fn second_snapshot(
        &mut self,
        allentities: AllEntities,
        master_leader: String,
    )
    {
        if master_leader == *"" {
            self.master_found = false;
            return
        } else {
            self.master_found = true;
        }
        trace!("Second snapshot: master_leader: {}, found: {}", master_leader, self.master_found);

        for entity in allentities.entities
            .iter()
            .filter(|r| r.hostname_port.as_ref().unwrap().clone() == master_leader.clone())
        {
            for keyspace in &entity.keyspaces {
                self.btreekeyspacediff
                    .entry(keyspace.keyspace_id.clone())
                    .and_modify(|keyspacediff| {
                        keyspacediff.second_keyspace_name = keyspace.keyspace_name.clone();
                        keyspacediff.second_keyspace_type = keyspace.keyspace_type.clone();
                        }
                    )
                    .or_insert(KeyspaceDiff {
                        second_keyspace_name: keyspace.keyspace_name.clone(),
                        second_keyspace_type: keyspace.keyspace_type.clone(),
                        ..Default::default()
                        }
                    );
            }
            for table in &entity.tables {
                self.btreetablesdiff
                    .entry(table.table_id.clone())
                    .and_modify(|tablediff| {
                        tablediff.second_keyspace_id = table.keyspace_id.clone();
                        tablediff.second_table_name = table.table_name.clone();
                        tablediff.second_state = table.state.clone();
                        }
                    )
                    .or_insert( TablesDiff {
                        second_keyspace_id: table.keyspace_id.clone(),
                        second_table_name: table.table_name.clone(),
                        second_state: table.state.clone(),
                        ..Default::default()
                    });
            }
            for tablet in &entity.tablets {
                self.btreetabletsdiff
                    .entry(tablet.tablet_id.clone())
                    .and_modify(|tabletdiff| {
                        tabletdiff.second_table_id = tablet.table_id.clone();
                        tabletdiff.second_state = tablet.state.clone();
                        tabletdiff.second_leader = tablet.leader.as_ref().unwrap_or(&"".to_string()).to_string();
                        }
                    )
                    .or_insert( TabletsDiff {
                        second_table_id: tablet.table_id.clone(),
                        second_state: tablet.state.clone(),
                        second_leader: tablet.leader.as_ref().unwrap_or(&"".to_string()).to_string(),
                        ..Default::default()
                    });
                for replica in tablet.replicas.clone().unwrap_or_default() {
                    self.btreereplicasdiff
                        .entry( (tablet.tablet_id.clone(), replica.server_uuid.clone()) )
                        .and_modify(|replicadiff| {
                            replicadiff.second_addr = replica.addr.clone();
                            replicadiff.second_replica_type = replica.replica_type.clone();
                            }
                        )
                        .or_insert( ReplicasDiff {
                            second_addr: replica.addr.clone(),
                            second_replica_type: replica.replica_type.clone(),
                            ..Default::default()
                        });
                }
            }
        }
    }
    pub async fn adhoc_read_first_snapshot(
        &mut self,
        hosts: &Vec<&str>,
        ports: &Vec<&str>,
        parallel: usize
    )
    {
        let allentities = AllEntities::read_entities(hosts, ports, parallel).await;
        let master_leader= AllIsLeader::return_leader_http(hosts, ports, parallel).await;
        self.first_snapshot(allentities, master_leader);
    }
    pub async fn adhoc_read_second_snapshot(
        &mut self,
        hosts: &Vec<&str>,
        ports: &Vec<&str>,
        parallel: usize
    )
    {
        let allentities = AllEntities::read_entities(hosts, ports, parallel).await;
        let master_leader= AllIsLeader::return_leader_http(hosts, ports, parallel).await;
        self.second_snapshot(allentities, master_leader);
    }
    pub fn print(
        &self,
    )
    {
        debug!("entering print function");
        if !self.master_found {
            println!("Master leader was not found in hosts specified, skipping entity diff.");
            return;
        }
        //let is_system_keyspace = |keyspace: &str| -> bool {
        //    matches!(keyspace, "00000000000000000000000000000001" |   // ycql system
        //                       "00000000000000000000000000000002" |   // ycql system_schema
        //                       "00000000000000000000000000000003" |   // ycql system_auth
        //                       "00000001000030008000000000000000" |   // ysql template1
        //                       "000033e5000030008000000000000000")    // ysql template0
        //};
        for (keyspace_id, keyspace_row) in &self.btreekeyspacediff {
            if keyspace_row.first_keyspace_name == keyspace_row.second_keyspace_name
                && keyspace_row.first_keyspace_type == keyspace_row.second_keyspace_type
            {
                // the keyspaces exist in both snapshots.
                // however, ysql keyspaces do not get deleted upon 'drop database'.
                // a dropped ysql keyspace can be detected by having zero tables.
                if keyspace_row.second_keyspace_type == "ysql"
                {
                    let first_snapshot_table_count = self.btreetablesdiff
                        .iter()
                        .filter(|(_k,v)| v.first_keyspace_id == keyspace_id.clone())
                        .count();
                    let second_snapshot_table_count = self.btreetablesdiff
                        .iter()
                        .filter(|(_k,v)| v.second_keyspace_id == keyspace_id.clone())
                        .count();
                    // first and second table count > 0: the keyspace was normally existing in both snapshots.
                    if first_snapshot_table_count > 0 && second_snapshot_table_count > 0
                        // first and second table count == 0: the keyspace was a deleted ysql keyspace in both snapshots.
                        || first_snapshot_table_count == 0 && second_snapshot_table_count == 0
                    {
                        continue;
                    }
                    else if first_snapshot_table_count > 0 && second_snapshot_table_count == 0
                    // the first table count is > 0 and the second table count is 0: this is a deleted ysql keyspace/database.
                    {
                        let colocation = if self.btreetabletsdiff.iter()
                            .any(|(_k,v)| v.first_table_id == format!("{}.colocated.parent.uuid", &keyspace_id))
                            && keyspace_row.first_keyspace_type == "ysql"
                        {
                            "[colocated]"
                        } else {
                            ""
                        };
                        println!("{} Database: {}.{}, id: {} {}",
                                 "-".to_string().red(),
                                 keyspace_row.first_keyspace_type,
                                 keyspace_row.first_keyspace_name,
                                 keyspace_id,
                                 colocation
                        );
                    } else if first_snapshot_table_count == 0 && second_snapshot_table_count > 0
                    // the first table count is 0 and the second table count is > 0: the deleted database got undeleted (?)
                    // this is not possible, hence error.
                    {
                        error!("ysql keyspace name: {}, id: {} table count was zero, now is: {}. This is not possible",
                            keyspace_id,
                            keyspace_row.second_keyspace_name,
                            second_snapshot_table_count,
                        );
                    };
                } else {
                    // this is not a ysql keyspace.
                    // this means no change happened to the keyspace
                    continue;
                }
            }
            // the first snapshot fields are empty, which means the second snapshot fields are filled out:
            // this is an added keyspace.
            else if keyspace_row.first_keyspace_name.is_empty()
                && keyspace_row.first_keyspace_type.is_empty()
            {
                let colocation = if self.btreetabletsdiff.iter()
                    .any(|(_k,v)| v.second_table_id == format!("{}.colocated.parent.uuid", &keyspace_id))
                    && keyspace_row.second_keyspace_type == "ysql"
                {
                    "[colocated]"
                } else {
                    ""
                };
                println!("{} Database: {}.{}, id: {} {}",
                         "+".to_string().green(),
                         keyspace_row.second_keyspace_type,
                         keyspace_row.second_keyspace_name,
                         keyspace_id,
                         colocation
                );
            }
            // the second snapshot fields are empty, which means the first snapshot fields are filled out:
            // this is a removed keyspace.
            // this has to be a removed ycql keyspace: ysql keyspaces do not get removed.
            else if keyspace_row.second_keyspace_name.is_empty()
                && keyspace_row.second_keyspace_type.is_empty()
            {
                let colocation = if self.btreetabletsdiff.iter()
                    .any(|(_k,v)| v.first_table_id == format!("{}.colocated.parent.uuid", &keyspace_id))
                    && keyspace_row.first_keyspace_type == "ysql"
                {
                    "[colocated]"
                } else {
                    ""
                };
                println!("{} Database: {}.{}, id: {} {}",
                         "-".to_string().red(),
                         keyspace_row.first_keyspace_type,
                         keyspace_row.first_keyspace_name,
                         keyspace_id,
                         colocation
                );
            } else {
                // at this point the fields between first and second snapshot are not alike.
                // this leaves one option: the keyspace name has changed.
                let colocation = if self.btreetabletsdiff.iter()
                    .any(|(_k,v)| v.first_table_id == format!("{}.colocated.parent.uuid", &keyspace_id))
                    && keyspace_row.first_keyspace_type == "ysql"
                {
                    "[colocated]"
                } else {
                    ""
                };
                println!("{} Database: {}.{}->{}, id: {} {}",
                         "=".to_string().yellow(),
                         keyspace_row.first_keyspace_type,
                         keyspace_row.first_keyspace_name.yellow(),
                         keyspace_row.second_keyspace_name.yellow(),
                         keyspace_id,
                         colocation
                );
            }
        }
        let object_oid_number = |oid: &str| -> u32 {
            // The oid entry normally is a 32 byte UUID for both ycql and ysql, which only contains hexadecimal numbers.
            // However, there is a single entry in system_schema that has a table_id that is 'sys.catalog.uuid'
            if oid.len() == 32_usize {
                let true_oid = &oid[24..];
                u32::from_str_radix(true_oid, 16).unwrap()
            } else {
                0
            }
        };
        for (table_id, table_row) in &self.btreetablesdiff {
            // skip system keyspaces
            //if is_system_keyspace(table_row.first_keyspace_id.as_str())
            //    || is_system_keyspace(table_row.second_keyspace_id.as_str())
            //{
            //    continue;
            //}
            if table_row.first_keyspace_id == table_row.second_keyspace_id
                && table_row.first_table_name  == table_row.second_table_name
                && table_row.first_state == table_row.second_state
            {
                // the table properties are exactly alike, so no reason to display anything: nothing has changed.
                continue;
            }
            // first snapshot fields are empty, which means second snapshot fields are filled out:
            // this is an added object.
            else if table_row.first_table_name.is_empty()
                && table_row.first_state.is_empty()
                && table_row.first_keyspace_id.is_empty()
            {
                // ysql table_id has got the OID number in it,
                // the below function takes that, and tests if it's below 16384.
                // ysql oid numbers below 16384 are system/catalog tables.
                if object_oid_number(table_id.as_str()) < 16384
                    && &self.btreekeyspacediff
                    .get(&table_row.second_keyspace_id.clone())
                    .map(|r| r.second_keyspace_type.clone())
                    .unwrap_or_default() == "ysql"
                {
                    continue;
                }
                // if the table is colocated, it means it does not have one or more tablets
                // directly linked to the table.
                // To check for colocation:
                // - the table keyspace type must be ysql.
                // - the must be OID >= 16384 (catalog tables do not have tablets).
                // - there are no tablets directly linked to the table.
                let colocation = if &self.btreekeyspacediff
                    .get(&table_row.second_keyspace_id.clone())
                    .map(|r| r.second_keyspace_type.clone())
                    .unwrap_or_default() == "ysql"
                    && object_oid_number(table_id.as_str()) >= 16384
                    && !self.btreetabletsdiff
                    .iter()
                    .any(|(_k,v)| v.second_table_id == table_id.clone())
                {
                    "[colocated]"
                }
                else
                {
                    ""
                };
                println!("{} Object:   {}.{}.{}, state: {}, id: {} {}",
                        "+".to_string().green(),
                        &self.btreekeyspacediff
                            .get(&table_row.second_keyspace_id)
                            .map(|r| r.second_keyspace_type.clone())
                            .unwrap_or_default(),
                        &self.btreekeyspacediff
                            .get(&table_row.second_keyspace_id)
                            .map(|r| r.second_keyspace_name.clone())
                            .unwrap_or_default(),
                        table_row.second_table_name,
                        table_row.second_state,
                        table_id,
                        colocation,
                );
            }
            // second snapshot fields are emtpy, which means first snapshot fields are filled out:
            // this is a removed object.
            else if table_row.second_table_name.is_empty()
                && table_row.second_state.is_empty()
                && table_row.second_keyspace_id.is_empty()
            {
                // ysql table_id has got the OID number in it,
                // the below function takes that, and tests if it's below 16384.
                // ysql oid numbers below 16384 are system/catalog tables.
                if object_oid_number(table_id.as_str()) < 16384
                    && &self.btreekeyspacediff
                    .get(&table_row.first_keyspace_id)
                    .map(|r| r.first_keyspace_type.clone())
                    .unwrap_or_default() == "ysql"
                {
                    continue;
                }
                // if the table is colocated, it means it does not have one or more tablets
                // directly linked to the table.
                // normally (non-colocated) one or more tablets are linked to a table.
                // catalog generally do not have tablets linked to it, hence the OID >= 16384 check.
                let colocation = if &self.btreekeyspacediff
                    .get(&table_row.first_keyspace_id.clone())
                    .map(|r| r.first_keyspace_type.clone())
                    .unwrap_or_default() == "ysql"
                    && object_oid_number(table_id.as_str()) >= 16384
                    && !self.btreetabletsdiff
                    .iter()
                    .any(|(_k,v)| v.first_table_id == table_id.clone())
                {
                    "[colocated]"
                }
                else
                {
                    ""
                };
                println!("{} Object:   {}.{}.{}, state: {}, id: {} {}",
                        "-".to_string().red(),
                        &self.btreekeyspacediff
                            .get(&table_row.first_keyspace_id)
                            .map(|r| r.first_keyspace_type.clone())
                            .unwrap_or_default(),
                        &self.btreekeyspacediff
                            .get(&table_row.first_keyspace_id)
                            .map(|r| r.first_keyspace_name.clone())
                            .unwrap_or_default(),
                        table_row.first_table_name,
                        table_row.first_state,
                        table_id,
                        colocation,
                );
            } else {
                // at this point the table properties are not alike.
                //
                // if the table is colocated, it means it does not have one or more tablets
                // directly linked to the table.
                // normally (non-colocated) one or more tablets are linked to a table.
                // catalog generally do not have tablets linked to it, hence the OID >= 16384 check.
                let colocation = if &self.btreekeyspacediff
                    .get(&table_row.first_keyspace_id.clone())
                    .map(|r| r.first_keyspace_type.clone())
                    .unwrap_or_default() == "ysql"
                    && object_oid_number(table_id.as_str()) >= 16384
                    && !self.btreetabletsdiff
                    .iter()
                    .any(|(_k,v)| v.first_table_id == table_id.clone())
                {
                    "[colocated]"
                }
                else
                {
                    ""
                };
                print!("{} Object:   {}.{}.",
                         "=".to_string().yellow(),
                         &self.btreekeyspacediff
                             .get(&table_row.first_keyspace_id)
                             .map(|r| r.first_keyspace_type.clone())
                             .unwrap_or_default(),
                         &self.btreekeyspacediff
                             .get(&table_row.first_keyspace_id)
                             .map(|r| r.first_keyspace_name.clone())
                             .unwrap_or_default(),
                );
                // table name different (alter table rename to)
                if table_row.first_table_name != table_row.second_table_name
                {
                    print!("{}->{}, ", table_row.first_table_name.yellow(), table_row.second_table_name.yellow());
                }
                else
                {
                    print!("{}, ", table_row.first_table_name);
                }
                if table_row.first_state != table_row.second_state
                {
                    print!("state: {}->{}, ", table_row.first_state.yellow(), table_row.second_state.yellow());
                }
                else
                {
                    print!("state: {}, ", table_row.first_state);
                }
                println!("id: {} {}",
                         table_id,
                         colocation,
                );
            }
        }
        for (tablet_id, tablet_row) in &self.btreetabletsdiff {
            if tablet_row.first_table_id == tablet_row.second_table_id
                && tablet_row.first_state == tablet_row.second_state
                && tablet_row.first_leader == tablet_row.second_leader
            {
                // the tablet data of the first and second snapshot is alike: nothing has changed.
                debug!("tablet: {}, state: {}-{}, leader: {}-{}", tablet_id, tablet_row.first_state, tablet_row.second_state, tablet_row.first_leader, tablet_row.second_leader);
                continue;
            }
            // first snapshot fields are empty, which means second snapshot fields are filled out:
            // this is an added tablet.
            if tablet_row.first_table_id.is_empty()
                && tablet_row.first_state.is_empty()
                && tablet_row.first_leader.is_empty()
            {
                println!("{} Tablet:   {}.{}.{}.{}, state: {}, leader: {}",
                    "+".to_string().green(),
                    self.btreetablesdiff
                        .get(&tablet_row.second_table_id)
                        .map(|r| {
                            self.btreekeyspacediff
                                .get(&r.second_keyspace_id)
                                .map(|r| r.second_keyspace_type.clone())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    self.btreetablesdiff
                        .get(&tablet_row.second_table_id)
                        .map(|r| {
                                 self.btreekeyspacediff
                                     .get(&r.second_keyspace_id.clone())
                                     .map(|r| r.second_keyspace_name.clone())
                                     .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    self.btreetablesdiff
                        .get(&tablet_row.second_table_id)
                        .map(|r| r.second_table_name.clone())
                        .unwrap_or_default(),
                    tablet_id,
                    tablet_row.second_state,
                    self.btreereplicasdiff
                        .iter()
                        .find(|((replica_tablet_id, replica_server_uuid), _replicadiff)| replica_tablet_id == tablet_id && replica_server_uuid.clone() == tablet_row.second_leader )
                        .map(|((_replica_tablet_id, _replica_server_uuid), replicadiff)| replicadiff.second_addr.clone())
                        .unwrap_or_default()
                );
            }
            // second snapshot fields are empty, which means first snapshot fields are filled out:
            // this is a deleted tablet object.
            else if tablet_row.second_table_id.is_empty()
                && tablet_row.second_state.is_empty()
                && tablet_row.second_leader.is_empty()
            {
                println!("{} Tablet:   {}.{}.{}.{}, state: {}, leader: {}",
                         "-".to_string().red(),
                         self.btreetablesdiff
                             .get(&tablet_row.first_table_id)
                             .map(|r| {
                                 self.btreekeyspacediff
                                     .get(&r.first_keyspace_id)
                                     .map(|r| r.first_keyspace_type.clone())
                                     .unwrap_or_default()
                             })
                             .unwrap_or_default(),
                         self.btreetablesdiff
                             .get(&tablet_row.first_table_id)
                             .map(|r| {
                                 self.btreekeyspacediff
                                     .get(&r.first_keyspace_id.clone())
                                     .map(|r| r.first_keyspace_name.clone())
                                     .unwrap_or_default()
                             })
                             .unwrap_or_default(),
                         self.btreetablesdiff
                             .get(&tablet_row.first_table_id)
                             .map(|r| r.first_table_name.clone())
                             .unwrap_or_default(),
                         tablet_id,
                         tablet_row.first_state,
                         self.btreereplicasdiff
                             .iter()
                             .find(|((replica_tablet_id, replica_server_uuid), _replicadiff)| replica_tablet_id == tablet_id && replica_server_uuid.clone() == tablet_row.first_leader )
                             .map(|((_replica_tablet_id, _replica_server_uuid), replicadiff)| replicadiff.first_addr.clone())
                             .unwrap_or_default()
                );
            } else {
                // at this point we know the tablets are not alike, but not added or removed.
                print!("{} Tablet:   {}.{}.{}.{}, ",
                    "=".to_string().yellow(),
                    self.btreetablesdiff
                        .get(&tablet_row.second_table_id)
                        .map(|r| {
                            self.btreekeyspacediff
                                .get(&r.second_keyspace_id)
                                .map(|r| r.second_keyspace_type.clone())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    self.btreetablesdiff
                        .get(&tablet_row.second_table_id)
                        .map(|r| {
                                 self.btreekeyspacediff
                                     .get(&r.second_keyspace_id.clone())
                                     .map(|r| r.second_keyspace_name.clone())
                                     .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                    self.btreetablesdiff
                        .get(&tablet_row.second_table_id)
                        .map(|r| r.second_table_name.clone())
                        .unwrap_or_default(),
                    tablet_id,
                );
                if tablet_row.first_state != tablet_row.second_state
                {
                    print!("state: {}->{}, ", tablet_row.first_state.yellow(), tablet_row.second_state.yellow());
                }
                else
                {
                    print!("state: {}", tablet_row.second_state);
                }
                if tablet_row.first_leader != tablet_row.second_leader
                {
                    println!(" leader: {}->{}",
                             self.btreereplicasdiff
                                 .iter()
                                 .find(|((replica_tablet_id, replica_server_uuid), _replicadiff)| replica_tablet_id == tablet_id && replica_server_uuid.clone() == tablet_row.first_leader )
                                 .map(|((_, _), replicadiff)| replicadiff.first_addr.clone())
                                 .unwrap_or_default()
                                 .yellow(),
                             self.btreereplicasdiff
                                 .iter()
                                 .find(|((replica_tablet_id, replica_server_uuid), _replicadiff)| replica_tablet_id == tablet_id && replica_server_uuid.clone() == tablet_row.second_leader )
                                 .map(|((_, _), replicadiff)| replicadiff.second_addr.clone())
                                 .unwrap_or_default()
                                 .yellow(),
                    );
                }
                else
                {
                    println!(" leader: {}",
                             self.btreereplicasdiff
                                 .iter()
                                 .find(|((replica_tablet_id, replica_server_uuid), _replicadiff)| replica_tablet_id == tablet_id && replica_server_uuid.clone() == tablet_row.second_leader )
                                 .map(|((_, _), replicadiff)| replicadiff.second_addr.clone())
                                 .unwrap_or_default(),
                    );
                }
            }
        }
        for ((tablet_id, _), replica_row) in &self.btreereplicasdiff
        {
            if replica_row.first_replica_type == replica_row.second_replica_type
                && replica_row.first_addr == replica_row.second_addr
            {
                // identical replica information: nothing changed.
                continue;
            }
            // if the first replica info is empty, it means a replica was added.
            if replica_row.first_replica_type.is_empty()
                && replica_row.first_addr.is_empty()
            {
                println!("{} Replica:  {}:{}.{}.{}.{}, Type: {}",
                    "+".to_string().green(),
                    replica_row.second_addr,
                    self.btreetabletsdiff
                        .get(&tablet_id.clone())
                        .map(|tablet| {
                            self.btreetablesdiff
                                .get(&tablet.second_table_id.clone())
                                .map(|table| {
                                    self.btreekeyspacediff
                                        .get(&table.second_keyspace_id)
                                        .map(|keyspace| keyspace.second_keyspace_type.clone())
                                        .unwrap_or_default()
                                })
                                .unwrap_or_default()
                        } )
                        .unwrap_or_default(),
                    self.btreetabletsdiff
                        .get(&tablet_id.clone())
                        .map(|tablet| {
                            self.btreetablesdiff
                                .get(&tablet.second_table_id.clone())
                                .map(|table| {
                                    self.btreekeyspacediff
                                        .get(&table.second_keyspace_id)
                                        .map(|keyspace| keyspace.second_keyspace_name.clone())
                                        .unwrap_or_default()
                                })
                                .unwrap_or_default()
                        } )
                        .unwrap_or_default(),
                    self.btreetabletsdiff
                        .get(&tablet_id.clone())
                        .map(|tablet| {
                            self.btreetablesdiff
                                .get(&tablet.second_table_id.clone())
                                .map(|table| table.second_table_name.clone())
                                .unwrap_or_default()
                        } )
                        .unwrap_or_default(),
                    tablet_id,
                    replica_row.second_replica_type,
                );
            }
            // if the second replica info is empty, it means a replica was removed.
            else if replica_row.second_replica_type.is_empty()
                && replica_row.second_addr.is_empty()
            {
                println!("{} Replica:  {}:{}.{}.{}.{}, Type: {}",
                         "-".to_string().red(),
                         replica_row.first_addr,
                         self.btreetabletsdiff
                             .get(&tablet_id.clone())
                             .map(|tablet| {
                                 self.btreetablesdiff
                                     .get(&tablet.first_table_id.clone())
                                     .map(|table| {
                                         self.btreekeyspacediff
                                             .get(&table.first_keyspace_id)
                                             .map(|keyspace| keyspace.first_keyspace_type.clone())
                                             .unwrap_or_default()
                                     })
                                     .unwrap_or_default()
                             } )
                             .unwrap_or_default(),
                         self.btreetabletsdiff
                             .get(&tablet_id.clone())
                             .map(|tablet| {
                                 self.btreetablesdiff
                                     .get(&tablet.first_table_id.clone())
                                     .map(|table| {
                                         self.btreekeyspacediff
                                             .get(&table.first_keyspace_id)
                                             .map(|keyspace| keyspace.first_keyspace_name.clone())
                                             .unwrap_or_default()
                                     })
                                     .unwrap_or_default()
                             } )
                             .unwrap_or_default(),
                         self.btreetabletsdiff
                             .get(&tablet_id.clone())
                             .map(|tablet| {
                                 self.btreetablesdiff
                                     .get(&tablet.first_table_id.clone())
                                     .map(|table| table.first_table_name.clone())
                                     .unwrap_or_default()
                             } )
                             .unwrap_or_default(),
                         tablet_id,
                         replica_row.first_replica_type,
                );
            }
            else
            {
                // the entries have changed?
                //println!("{} Replica: {}.{}.{}.{}.{}, Type: {}",
                print!("{} Replica: ",
                    "=".to_string().yellow(),
                );
                if replica_row.first_addr != replica_row.second_addr
                {
                    print!("{}->{}:",
                        replica_row.first_addr.yellow(),
                        replica_row.second_addr.yellow(),
                    );
                }
                else
                {
                    print!("{}:",
                        replica_row.second_addr,
                    );
                }
                print!("{}.{}.{}.{}, ",
                    self.btreetabletsdiff
                        .get(&tablet_id.clone())
                        .map(|tablet| {
                            self.btreetablesdiff
                                .get(&tablet.second_table_id.clone())
                                .map(|table| {
                                    self.btreekeyspacediff
                                        .get(&table.second_keyspace_id)
                                        .map(|keyspace| keyspace.second_keyspace_type.clone())
                                        .unwrap_or_default()
                                })
                                .unwrap_or_default()
                        } )
                        .unwrap_or_default(),
                    self.btreetabletsdiff
                        .get(&tablet_id.clone())
                        .map(|tablet| {
                            self.btreetablesdiff
                                .get(&tablet.second_table_id.clone())
                                .map(|table| {
                                    self.btreekeyspacediff
                                        .get(&table.second_keyspace_id)
                                        .map(|keyspace| keyspace.second_keyspace_name.clone())
                                        .unwrap_or_default()
                                })
                                .unwrap_or_default()
                        } )
                        .unwrap_or_default(),
                    self.btreetabletsdiff
                        .get(&tablet_id.clone())
                        .map(|tablet| {
                            self.btreetablesdiff
                                .get(&tablet.second_table_id.clone())
                                .map(|table| table.second_table_name.clone())
                                .unwrap_or_default()
                        } )
                        .unwrap_or_default(),
                    tablet_id,
                );
                if replica_row.first_replica_type != replica_row.second_replica_type
                {
                    println!("Type: {}->{}",
                    replica_row.first_replica_type.yellow(),
                    replica_row.second_replica_type.yellow(),
                    );
                }
                else
                {
                    println!("Type: {}", replica_row.second_replica_type);
                };
            };
        }
    }
}

pub async fn entity_diff(
    options: &Opts,
) -> Result<()>
{
    info!("entity diff");
    if options.begin.is_none() || options.end.is_none() {
        snapshot::Snapshot::print()?;
    }
    if options.snapshot_list { return Ok(()) };
    let (begin_snapshot, end_snapshot, _begin_snapshot_row) = snapshot::Snapshot::read_begin_end_snapshot_from_user(options.begin, options.end)?;

    let entity_diff = EntitiesDiff::snapshot_diff(&begin_snapshot, &end_snapshot)?;
    entity_diff.print();

    Ok(())
}

pub async fn print_entities(
    hosts: Vec<&str>,
    ports: Vec<&str>,
    parallel: usize,
    options: &Opts,
) -> Result<()>
{
    let table_name_filter = utility::set_regex(&options.table_name_match);
    let hostname_filter = utility::set_regex(&options.hostname_match);

    match options.print_entities.as_ref().unwrap()
    {
        Some(snapshot_number) =>
        {
            let mut allentities = AllEntities::new();
            allentities.entities = snapshot::read_snapshot_json(snapshot_number, "entities")?;
            let leader_hostname = AllIsLeader::return_leader_snapshot(snapshot_number)?;
            let (dead_nodes, under_replicated_tablets) = AllHealthCheck::return_dead_nodes_and_under_replicated_tablets_snapshot(snapshot_number, &leader_hostname)?;
            allentities.print(&table_name_filter, &options.details_enable, leader_hostname, &hostname_filter, dead_nodes, under_replicated_tablets)?;
        },
        None =>
        {
            let allentities = AllEntities::read_entities(&hosts, &ports, parallel).await;
            let leader_hostname = AllIsLeader::return_leader_http(&hosts, &ports, parallel).await;
            let (dead_nodes, under_replicated_tablets) = AllHealthCheck::return_dead_nodes_and_under_replicated_tablets_http(&hosts, &ports, parallel, &leader_hostname).await?;
            allentities.print(&table_name_filter, &options.details_enable, leader_hostname, &hostname_filter, dead_nodes, under_replicated_tablets)?;
        },
    }
    Ok(())
}

pub async fn print_coloc_leader_host(
    hosts: Vec<&str>,
    ports: Vec<&str>,
    parallel: usize,
    options: &Opts,
) -> Result<()>
{
    let colocated_database = &options.get_coloc_leader_host.as_ref().unwrap();

    let allentities = AllEntities::read_entities(&hosts, &ports, parallel).await;
    let leader_hostname = AllIsLeader::return_leader_http(&hosts, &ports, parallel).await;
    allentities.print_coloc_leader_host(leader_hostname, colocated_database)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_parse_simple_entities_dump() {
        let json = r#"
{
  "keyspaces": [
    {
      "keyspace_id": "00000000000000000000000000000001",
      "keyspace_name": "system",
      "keyspace_type": "ycql"
    }
  ],
  "tables": [
    {
      "table_id": "000000010000300080000000000000af",
      "keyspace_id": "00000001000030008000000000000000",
      "table_name": "pg_user_mapping_user_server_index",
      "state": "RUNNING"
    }
  ],
  "tablets": [
    {
      "table_id": "sys.catalog.uuid",
      "tablet_id": "00000000000000000000000000000000",
      "state": "RUNNING"
    },
    {
      "table_id": "a1da3fb4b3be4bd4860253e723d11b97",
      "tablet_id": "235b5b031f094ec3bf6be2a023abebba",
      "state": "RUNNING",
      "replicas": [
        {
          "type": "VOTER",
          "server_uuid": "5b6fd994d7e34504ac48a5e653456704",
          "addr": "yb-3.local:9100"
        },
        {
          "type": "VOTER",
          "server_uuid": "a3f5a16532bb4ed4a061e794831168f8",
          "addr": "yb-1.local:9100"
        },
        {
          "type": "VOTER",
          "server_uuid": "e7a4a66ae7f94eb6a75b0ce3a90ab5ba",
          "addr": "yb-2.local:9100"
        }
      ],
      "leader": "a3f5a16532bb4ed4a061e794831168f8"
    }
  ]
}
        "#.to_string();
        let result = AllEntities::parse_entities(json, "", "");
        assert_eq!(result.keyspaces[0].keyspace_type,"ycql");
        assert_eq!(result.tables[0].table_name,"pg_user_mapping_user_server_index");
        assert_eq!(result.tablets[0].table_id,"sys.catalog.uuid");
        assert_eq!(result.tablets[1].table_id,"a1da3fb4b3be4bd4860253e723d11b97");
        assert_eq!(result.tablets[1].replicas.as_ref().unwrap()[0].server_uuid,"5b6fd994d7e34504ac48a5e653456704");
        assert_eq!(result.tablets[1].leader.as_ref().unwrap(),"a3f5a16532bb4ed4a061e794831168f8");
    }

    #[test]
    fn integration_parse_entities() {
        let hostname = utility::get_hostname_master();
        let port = utility::get_port_master();

        let entities = AllEntities::read_http(&hostname, &port);

        assert!(!entities.keyspaces.is_empty());
        assert!(!entities.tables.is_empty());
        assert!(!entities.tablets.is_empty());
    }

}