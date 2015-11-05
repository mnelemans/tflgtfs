#![feature(custom_derive)]

extern crate hyper;
extern crate rustc_serialize;
extern crate scoped_threadpool;
extern crate csv;

use std::fs;
use std::sync::Arc;
use std::path::Path;
use std::fs::File;
use std::io::{Read, Write};
use std::collections::{HashSet, HashMap};

use hyper::client::Client;
use hyper::header::{Accept, qitem};
use hyper::mime::{Mime, TopLevel, SubLevel};

use rustc_serialize::json;

use scoped_threadpool::Pool;

#[derive(Clone)]
struct MyClient {
    client : Arc<Client>,
    app_id : String,
    app_key : String,
    cache_dir : String,
}

#[derive(Clone, Debug, RustcDecodable)]
struct Line {
    id : String,
    name : String,
    modeName : String,
    routeSections : Vec<RouteSection>,
    stops : Option<Vec<Stop>>,
}

#[derive(Clone, Debug, RustcDecodable)]
struct Stop {
    naptanId : String,
    commonName : String,
    lat : f64,
    lon : f64,
    children : Vec<Stop>,
}

#[derive(Clone, Debug, RustcDecodable)]
struct RouteSection {
    name : String,
    direction : String,
    originator : String,
    destination : String,
    timetable : Option<TimeTable>,
}

#[derive(Clone, Debug, RustcDecodable)]
struct Interval {
    stopId : String, 
    timeToArrival: f64,
}

#[derive(Clone, Debug, RustcDecodable)]
struct StationInterval {
    id : i64,
    intervals : Vec<Interval>
}

#[derive(Clone, Debug, RustcDecodable)]
struct KnownJourney {
    intervalId : i64,
    hour : String,
    minute : String,
}

#[derive(Clone, Debug, RustcDecodable)]
struct Schedule {
    name : String,
    knownJourneys : Vec<KnownJourney>,
}

#[derive(Clone, Debug, RustcDecodable)]
struct TimeTable {
    stationIntervals : Vec<StationInterval>,
    schedules : Vec<Schedule>,
}

#[derive(Clone, Debug, RustcDecodable)]
struct RoutesTimeTables {
    routes : Vec<TimeTable>,
}

#[derive(Debug, RustcDecodable)]
struct TimeTableResponse {
    timetable : RoutesTimeTables,
}

impl MyClient {
    fn new() -> MyClient {
        let cache_path : &Path = Path::new("./cache");
        fs::create_dir(cache_path).unwrap();
        return MyClient{
            client : Arc::new(Client::new()),
            app_id : String::new(),
            app_key : String::new(),
            cache_dir : String::from("./cache"),
        }
    }

    fn get(&self, endpoint : &str) -> String {
        match self.cache_get(endpoint) {
            Some(body) => body,
            None => self.remote_get(endpoint)
        }
    }

    fn remote_get(&self, endpoint : &str) -> String {
        let req_uri = format!("https://api.tfl.gov.uk{}?app_id={}&app_key={}", endpoint, self.app_id, self.app_key);
        let mut body = String::new();
        let mut resp = self.client.get(&req_uri)
            .header(Accept(vec![
                qitem(Mime(TopLevel::Application,
                    SubLevel::Ext("json".to_owned()), vec![])),
            ]))
            .send().unwrap();
        resp.read_to_string(&mut body).unwrap();
        self.cache_put(endpoint, body)
    }

    fn cache_fname(&self, endpoint : &str) -> String {
        let fname = String::from(endpoint);
        let fname0 = fname.replace("/", "_");
        self.cache_dir.clone() + "/" + &fname0
    }

    fn cache_put(&self, endpoint : &str, body : String) -> String {
        let mut f = File::create(self.cache_fname(endpoint)).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        body
    }

    fn cache_get(&self, endpoint : &str) -> Option<String> {
        let mut body = String::new();
        match File::open(self.cache_fname(endpoint)) {
            Ok(ref mut f) => {
                f.read_to_string(&mut body).unwrap();
                Some(body)
            },
            Err(_) => None,
        }
    }
}

fn get_lines(client : &MyClient) -> Vec<Line> {
    let body = client.get("/line/route");
    json::decode(&body).unwrap()
}

fn get_timetable(client : &MyClient, line_id : &str, originator: &str, destination : &str) -> Option<TimeTable> {
    let req_uri = format!("/line/{}/timetable/{}/to/{}", line_id, originator, destination);
    let body = client.get(&req_uri);
    match json::decode::<TimeTableResponse>(&body) {
        Ok(ttresp) =>  Some(ttresp.timetable.routes[0].clone()),
        Err(err) => {
            println!("Error decoding timetable {}", err);
            None
        },
    }
}

fn get_stops(client : &MyClient, line_id : &str) -> Vec<Stop> {
    let req_uri = format!("/line/{}/stoppoints", line_id);
    let body = client.get(&req_uri);
    match json::decode::<Vec<Stop>>(&body) {
        Ok(stops) => stops,
        Err(err) => {
            println!("Error decoding stops: {}", err);
            Vec::<Stop>::new()
        }
    }
}

fn route_section_id(line : &Line, section : &RouteSection) -> String {
    return line.id.clone() + " " + &section.originator + " to " + &section.destination;
}

fn write_agency(gtfs_path : &str) {
    let fname = format!("{}/{}", gtfs_path, "/agency.txt");
    let fpath = Path::new(&fname);
    let mut wtr = csv::Writer::from_file(fpath).unwrap();
    let records = vec![
        ("agency_id","agency_name","agency_url","agency_timezone"),
        ("tfl","Transport For London","https://tfl.gov.uk","Europe/London")
    ];
    for record in records {
        wtr.encode(record).unwrap();
    }
}

fn route_type(line : &Line) -> &'static str {
    match &line.modeName[..] {
        "dlr" | "tram" => "0",
        "tube" | "overground" => "1",
        "national-rail" | "tflrail" => "2",
        "bus" => "3",
        "river-tour" | "river-bus" => "4",
        "cable-car" => "5",
        _ => {
            println!("Missing line modeName match: {}", line.modeName);
            ""
        },
    }
}

fn write_routes(gtfs_path : &str, lines : &Vec<Line>) {
    let fname = format!("{}/{}", gtfs_path, "/routes.txt");
    let fpath = Path::new(&fname);
    let mut wtr = csv::Writer::from_file(fpath).unwrap();
    wtr.encode(("route_id", "agency_id", "route_short_name", "route_long_name", "route_type")).unwrap();
    for line in lines {
        wtr.encode((&line.id, "tfl", &line.name, "", route_type(&line))).unwrap();
    }
}

fn write_stops(gtfs_path : &str, lines : &Vec<Line>) {
    let fname = format!("{}/{}", gtfs_path, "/stops.txt");
    let fpath = Path::new(&fname);
    let mut wtr = csv::Writer::from_file(fpath).unwrap();
    let mut written_stops = HashSet::<String>::new();
    wtr.encode(("stop_id", "stop_name", "stop_lat", "stop_lon")).unwrap();
    for line in lines {
        let stops = line.stops.as_ref().unwrap();
        for stop in stops {
            match written_stops.contains(&stop.naptanId) {
                true => (),
                false => {
                    wtr.encode((stop.naptanId.clone(), stop.commonName.clone(), stop.lat, stop.lon)).unwrap();
                    written_stops.insert(stop.naptanId.clone());
                    for child in &stop.children {
                        match written_stops.contains(&child.naptanId) {
                            true => (),
                            false => {
                                wtr.encode((child.naptanId.clone(), child.commonName.clone(), stop.lat, stop.lon)).unwrap();
                                written_stops.insert(child.naptanId.clone());
                            },
                        }
                    }
                },
            };
        }
    }
}

fn write_calendar(gtfs_path : &str) {
    let fname = format!("{}/{}", gtfs_path, "/calendar.txt");
    let fpath = Path::new(&fname);
    let mut wtr = csv::Writer::from_file(fpath).unwrap();
    let start_date = "20151031";
    let end_date = "20161031";
    let records = vec![
        ("service_id", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "start_date", "end_date"),
        ("School Monday", "1", "0", "0", "0", "0", "0", "0", &start_date, &end_date),
        ("Sunday Night/Monday Morning", "1", "0", "0", "0", "0", "0", "1", &start_date, &end_date),
        ("School Monday, Tuesday, Thursday & Friday", "1", "1", "0", "1", "1", "0", "0", &start_date, &end_date),
        ("Tuesday", "0", "1", "0", "0", "0", "0", "0", &start_date, &end_date),
        ("Monday - Thursday", "1", "1", "1", "1", "0", "0", "0", &start_date, &end_date),
        ("Saturday", "0", "0", "0", "0", "0", "0", "1", &start_date, &end_date),
        ("Saturday and Sunday", "0", "0", "0", "0", "0", "1","1", &start_date, &end_date),
        ("Sunday", "0", "0", "0", "0", "0", "0", "1", &start_date, &end_date),
        ("School Tuesday", "0", "1", "0", "0", "0", "0", "0", &start_date, &end_date),
        ("Saturday Night/Sunday Morning", "0", "0", "0", "0", "0", "1", "1", &start_date, &end_date),
        ("Mo-Fr Night/Tu-Sat Morning", "1", "1", "1", "1","1", "1", "0", &start_date, &end_date),
        ("Monday to Thursday", "1", "1", "1", "1", "0", "0", "0", &start_date, &end_date),
        ("Mo-Th Nights/Tu-Fr Morning", "1", "1", "1", "1", "1", "0", "0", &start_date, &end_date),
        ("Saturday (also Good Friday)", "0", "0", "0", "0", "0", "1", "0", &start_date, &end_date),
        ("Mon-Th Schooldays", "1", "1", "1", "1", "0", "0", "0", &start_date, &end_date),
        ("Saturdays and Public Holidays", "0", "0", "0", "0", "0", "1", "0", &start_date, &end_date),
        ("Friday Night/Saturday Morning", "0", "0", "0", "0", "1", "1", "0", &start_date, &end_date),
        ("Friday", "0", "0", "0", "0", "1", "0", "0", &start_date, &end_date),
        ("Thursdays", "0", "0", "0", "1", "0", "0", "0", &start_date, &end_date),
        ("Sunday night/Monday morning - Thursday night/Friday morning", "1", "1", "1", "1", "1", "0", "1", &start_date, &end_date),
        ("School Thursday", "0", "0", "0", "1", "0", "0", "0", &start_date, &end_date),
        ("School Friday", "0", "0", "0", "0", "1", "0", "0", &start_date, &end_date),
        ("Daily", "1", "1", "1", "1", "1", "1", "1", &start_date, &end_date),
        ("Tuesday, Wednesday & Thursday", "0", "1", "1", "1", "0", "0", "0", &start_date, &end_date),
        ("Mon-Fri Schooldays", "1", "1", "1", "1", "1", "0", "0", &start_date, &end_date),
        ("Wednesday", "0", "0", "1", "0", "0", "0", "0", &start_date, &end_date),
        ("Monday, Tuesday and Thursday", "1", "1", "0", "1", "0", "0", "0", &start_date, &end_date),
        ("Wednesdays", "0", "0", "1", "0", "0", "0", "0", &start_date, &end_date),
        ("Monday to Friday", "1", "1", "1", "1", "1", "0", "0", &start_date, &end_date),
        ("Monday", "1", "0", "0", "0", "0", "0", "0", &start_date, &end_date),
        ("Sunday and other Public Holidays", "0", "0", "0", "0", "0", "0", "1", &start_date, &end_date),
        ("School Wednesday", "0", "0", "1", "0", "0", "0", "0", &start_date, &end_date),
        ("Monday - Friday", "1", "1", "1", "1", "1", "0", "0", &start_date, &end_date),
    ];
    for record in records {
        wtr.encode(record).unwrap();
    }
}

fn trip_id(line : &Line, section : &RouteSection, schedule : &Schedule, journey : &KnownJourney) -> String {
    let tfmt = time_offset_fmt(journey, 0.0);
    format!("{} {} to {} scheduled {} departs {}", line.id, section.originator, section.destination, schedule.name, tfmt)
}

fn write_route_section_trips(wtr : &mut csv::Writer<File>, line : &Line, section : &RouteSection) {
    let mut written_trips : HashSet<String> = HashSet::new();
    match section.timetable.as_ref() {
        None => (),
        Some(timetable) => {
            for schedule in &timetable.schedules {
                for journey in &schedule.knownJourneys {
                    let id = trip_id(line, section, schedule, journey);
                    match written_trips.contains(&id) {
                        true => (),
                        false => {
                            written_trips.insert(id.clone());
                            wtr.encode((&line.id, &schedule.name, trip_id(line, section, schedule, journey))).unwrap();
                        },
                    }
                }
            }
        },
    }
}

fn write_trips(gtfs_path : &str, lines : &Vec<Line>) {
    let fname = format!("{}/{}", gtfs_path, "/trips.txt");
    let fpath = Path::new(&fname);
    let mut wtr = csv::Writer::from_file(fpath).unwrap();
    wtr.encode(("route_id", "service_id", "trip_id")).unwrap();
    for line in lines {
        let mut written_route_sections = HashSet::<String>::new();
        let route_sections = &line.routeSections;
        for route_section in route_sections {
            let id = route_section_id(line, route_section);
            match written_route_sections.contains(&id) {
                true => (),
                false => {
                    write_route_section_trips(&mut wtr, line, route_section);
                    written_route_sections.insert(id);
                },
            };
        }
    }
}
fn time_offset_fmt(journey : &KnownJourney, offset : f64) -> String {
    let dep_hour : u64 = journey.hour.parse().unwrap();
    let dep_minute : u64 = journey.minute.parse().unwrap();
    let rounded_offset : u64 = offset.floor() as u64;
    let minute_offset : u64 = dep_minute + rounded_offset;
    let hour : u64 = dep_hour + minute_offset / 60;
    let minute : u64 = minute_offset % 60;
    format!("{:02}:{:02}", hour, minute) 
}

fn write_journey_stop_times(wtr : &mut csv::Writer<File>, line : &Line, section : &RouteSection, schedule : &Schedule, journey : &KnownJourney, interval : &StationInterval) {
    let mut stop_seq = 1;
    let trip_id = trip_id(line, section, schedule, journey);
    let dep_time = time_offset_fmt(journey, 0.0);
    wtr.encode((&trip_id, &section.originator, stop_seq, &dep_time, &dep_time)).unwrap();
    for stop in &interval.intervals {
        stop_seq += 1;
        let dep_time = time_offset_fmt(journey, stop.timeToArrival);
        wtr.encode((&trip_id, &stop.stopId, stop_seq, &dep_time, &dep_time)).unwrap();
    }
}

fn write_route_section_stop_times(wtr : &mut csv::Writer<File>, line : &Line, section : &RouteSection) {
    let mut written_trips : HashSet<String> = HashSet::new();
    match section.timetable.as_ref() {
        None => (),
        Some(timetable) => {
            let mut intervals : HashMap<i64, &StationInterval> = HashMap::new();
            for interval in &timetable.stationIntervals {
                intervals.insert(interval.id, interval);
            }
            for schedule in &timetable.schedules {
                for journey in &schedule.knownJourneys {
                    match intervals.get(&journey.intervalId) {
                        Some(interval) =>  {
                            let id = trip_id(line, section, schedule, journey);
                            match written_trips.contains(&id) {
                                true => (),
                                false => {
                                    written_trips.insert(id.clone());
                                    write_journey_stop_times(wtr, line, section, schedule, journey, interval);
                                }
                            }
                        },
                        None => println!("Error, Could not find interval for schedule!!!!"),
                    };
                }
            };
        },
    }

}

fn write_stop_times(gtfs_path : &str, lines : &Vec<Line>) {
    let fname = format!("{}/{}", gtfs_path, "/stop_times.txt");
    let fpath = Path::new(&fname);
    let mut wtr = csv::Writer::from_file(fpath).unwrap();
    wtr.encode(("trip_id", "stop_id", "stop_sequence", "arrival_time", "departure_time")).unwrap();
    for line in lines {
        let mut written_route_sections = HashSet::<String>::new();
        let route_sections = &line.routeSections;
        for route_section in route_sections {
            let id = route_section_id(line, route_section);
            match written_route_sections.contains(&id) {
                true => (),
                false => {
                    write_route_section_stop_times(&mut wtr, line, route_section);
                    written_route_sections.insert(id);
                },
            };
        }
    }
}

fn write_gtfs(lines : &Vec<Line>) {
        let gtfs_path : &Path = Path::new("./gtfs");
        let gtfs_path_str = gtfs_path.to_str().unwrap();
        fs::create_dir(gtfs_path_str).unwrap();
        write_agency(gtfs_path_str);
        write_routes(gtfs_path_str, lines);
        write_stops(gtfs_path_str, lines);
        write_calendar(gtfs_path_str);
        write_trips(gtfs_path_str, lines);
        write_stop_times(gtfs_path_str, lines);
}

fn main() {
    // Fetch data
    let client = Arc::new(MyClient::new());
    let mut lines = get_lines(&client);
    let mut pool = Pool::new(10);

    pool.scoped(|scope| {
        for line in &mut lines {
            let client = client.clone();
            scope.execute(move || {
                line.stops = Some(get_stops(&client, &line.id));
                for route_section in &mut line.routeSections {
                    println!("Getting Timetable for Line: {}, Route Section: {} ...", line.name, route_section.name);
                    route_section.timetable = get_timetable(&client, &line.id, &route_section.originator, &route_section.destination);
                }
            });
        }
    });

    // Generate a report
    let mut line_count = 0;
    let mut line_ids : HashSet<String> = HashSet::new();
    let mut route_section_count = 0;
    let mut route_section_ids: HashSet<String> = HashSet::new();
    let mut schedule_names: HashSet<String> = HashSet::new();
    for line in &lines {
        println!("{}, Duplicate: {}", line.id, line_ids.contains(&line.id));
        for route_section in &line.routeSections {
            let has_timetable = match route_section.timetable {
                Some(ref timetable) => {
                    for schedule in &timetable.schedules {
                        schedule_names.insert(schedule.name.clone());
                    }
                    true
                },
                None => false,
            };
            let id = route_section_id(&line, &route_section);
            println!("\t{}, Has Timetable: {}, Duplicate: {}", id, has_timetable, route_section_ids.contains(&id));
            route_section_ids.insert(id.clone());
            route_section_count += 1;
        }
        line_count += 1;
        line_ids.insert(line.id.clone());
    }
    println!("Duplicate Lines: {}, Duplicate Route Sections: {}", line_count-line_ids.len(), route_section_count-route_section_ids.len());

    println!("Schedule Names:");
    for schedule_name in &schedule_names {
        println!("\t{}", schedule_name);
    }

    // Generate CSV files from fetched data
    write_gtfs(&lines);
}
