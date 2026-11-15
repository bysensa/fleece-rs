//! Geometries to and from `geo-types`, and the algorithms of `geo` on what a document holds.
#![cfg(feature = "geo")]

use fleece::{Doc, Geometry, Object, Point, Polygon, Value, decode, encode};
use geo::{Area, BoundingRect, Centroid, Contains};
use geo_types::{
    Geometry as Geo, GeometryCollection, Line, LineString, MultiLineString, MultiPoint,
    MultiPolygon, Rect, Triangle, coord, line_string, point, polygon,
};
use proptest::collection::vec;
use proptest::prelude::*;

/// A square of 4 by 4 with a hole of 1 by 1 in it.
fn square_with_a_hole() -> geo_types::Polygon<f64> {
    polygon![
        exterior: [(x: 0.0, y: 0.0), (x: 4.0, y: 0.0), (x: 4.0, y: 4.0), (x: 0.0, y: 4.0)],
        interiors: [[(x: 2.0, y: 2.0), (x: 3.0, y: 2.0), (x: 3.0, y: 3.0), (x: 2.0, y: 3.0)]],
    ]
}

#[test]
fn every_shape_of_geo_types_is_a_geometry_and_comes_back() {
    let line = line_string![(x: 0.0, y: 0.0), (x: 1.0, y: 2.0), (x: -3.5, y: 0.25)];
    let shapes = [
        Geo::Point(point!(x: 1.5, y: -2.0)),
        Geo::LineString(line.clone()),
        Geo::LineString(LineString(vec![])),
        Geo::Polygon(square_with_a_hole()),
        Geo::MultiPoint(MultiPoint(vec![point!(x: 0.0, y: 0.0), point!(x: 1.0, y: 1.0)])),
        Geo::MultiLineString(MultiLineString(vec![line.clone(), line.clone()])),
        Geo::MultiPolygon(MultiPolygon(vec![square_with_a_hole(), square_with_a_hole()])),
    ];
    let collection = Geo::GeometryCollection(GeometryCollection(shapes.to_vec()));
    let nested = Geo::GeometryCollection(GeometryCollection(vec![collection.clone()]));
    for shape in shapes.iter().chain([&collection, &nested]) {
        let geometry = Geometry::from(shape);
        assert_eq!(&Geo::from(&geometry), shape);
        // And through the bytes of a document.
        let bytes = encode(&Value::Geometry(geometry.clone())).unwrap();
        assert_eq!(decode(&bytes).unwrap(), Value::Geometry(geometry));
        assert_eq!(Doc::new(&bytes).unwrap().root().as_geo().as_ref(), Some(shape));
    }
}

#[test]
fn the_shapes_a_geometry_has_not_become_the_ones_it_has() {
    let segment = Line::new(coord! { x: 0.0, y: 0.0 }, coord! { x: 3.0, y: 4.0 });
    let as_line = Geometry::Line(vec![Point { x: 0.0, y: 0.0 }, Point { x: 3.0, y: 4.0 }]);
    assert_eq!(Geometry::from(Geo::Line(segment)), as_line);

    let rect = Rect::new(coord! { x: 0.0, y: 0.0 }, coord! { x: 2.0, y: 1.0 });
    let Geometry::Polygon(sides) = Geometry::from(Geo::Rect(rect)) else { panic!() };
    assert_eq!((sides.exterior.len(), sides.interiors.len()), (5, 0));
    assert_eq!(Geo::from(Geometry::Polygon(sides)).unsigned_area(), 2.0);

    let triangle = Triangle::new(
        coord! { x: 0.0, y: 0.0 },
        coord! { x: 2.0, y: 0.0 },
        coord! { x: 0.0, y: 2.0 },
    );
    let Geometry::Polygon(sides) = Geometry::from(Geo::Triangle(triangle)) else { panic!() };
    assert_eq!(sides.exterior.len(), 4);
    assert_eq!(Geo::from(Geometry::Polygon(sides)).unsigned_area(), 2.0);
}

#[test]
fn the_algorithms_of_geo_work_on_what_a_document_holds() {
    let place = Value::Object(Object::from([
        ("name".to_owned(), Value::from("Moulinsart")),
        ("grounds".to_owned(), Value::Geometry(Geo::Polygon(square_with_a_hole()).into())),
        ("gate".to_owned(), Value::Geometry(Geometry::Point(Point { x: 1.0, y: 1.0 }))),
    ]));
    let bytes = encode(&place).unwrap();
    let fields = Doc::new(&bytes).unwrap().root().as_dict().unwrap();

    // Only the field asked for is decoded, and what comes out is a type of geo.
    let grounds = fields.get("grounds").unwrap().as_geo().unwrap();
    assert_eq!(grounds.unsigned_area(), 15.0);
    let bounds = grounds.bounding_rect().unwrap();
    assert_eq!(
        (bounds.min(), bounds.max()),
        (coord! { x: 0.0, y: 0.0 }, coord! { x: 4.0, y: 4.0 })
    );
    let Geo::Point(gate) = fields.get("gate").unwrap().as_geo().unwrap() else { panic!() };
    assert!(grounds.contains(&gate));
    assert!(!grounds.contains(&point!(x: 2.5, y: 2.5)), "the hole is not the grounds");
    assert!(!grounds.contains(&point!(x: 9.0, y: 9.0)));
    assert_eq!(grounds.centroid().map(|c| c.x() > 1.9 && c.x() < 2.0), Some(true));

    // What geo computes goes back into a document.
    let stored = encode(&Value::Geometry(Geo::Rect(bounds).into())).unwrap();
    let outline = Doc::new(&stored).unwrap().root().as_geo().unwrap();
    assert_eq!(outline.unsigned_area(), 16.0);

    // A value that is not a geometry is none.
    assert!(fields.get("name").unwrap().as_geo().is_none());
    assert!(fields.get("name").unwrap().as_geometry().is_none());
    assert_eq!(
        fields.get("gate").unwrap().as_geometry(),
        Some(Geometry::Point(Point { x: 1.0, y: 1.0 }))
    );
}

#[test]
fn geo_types_closes_the_rings_of_a_polygon() {
    let corner = |x: f64, y: f64| Point { x, y };
    let open = vec![corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0)];
    let polygon = Polygon { exterior: open.clone(), interiors: vec![open.clone()] };
    let Geometry::Polygon(back) = Geometry::from(Geo::from(Geometry::Polygon(polygon))) else {
        panic!()
    };
    // The first point of each ring is at its end as well.
    let closed = [open.clone(), vec![corner(0.0, 0.0)]].concat();
    assert_eq!(back, Polygon { exterior: closed.clone(), interiors: vec![closed] });
}

/// A geometry given away is not copied: the coordinates stay where they are in memory,
/// each way. This rests on the standard library collecting a vector into one of items of
/// the same size in place, which it does and does not promise; should a toolchain stop,
/// this test says so, and the conversions are then a copy and still right.
#[test]
fn a_geometry_given_away_keeps_its_memory() {
    let points: Vec<Point> = (0..1000).map(|i| Point { x: f64::from(i), y: 0.5 }).collect();
    let rings: Vec<Vec<Point>> = (0..3).map(|_| points.clone()).collect();
    let (at, rings_at) = (points.as_ptr() as usize, rings[1].as_ptr() as usize);

    let Geo::LineString(line) = Geo::from(Geometry::Line(points)) else { panic!() };
    assert_eq!(line.0.as_ptr() as usize, at);
    let Geometry::Line(points) = Geometry::from(Geo::LineString(line)) else { panic!() };
    assert_eq!(points.as_ptr() as usize, at);

    let Geo::MultiLineString(lines) = Geo::from(Geometry::MultiLine(rings)) else { panic!() };
    assert_eq!(lines.0[1].0.as_ptr() as usize, rings_at);
    let Geometry::MultiLine(rings) = Geometry::from(Geo::MultiLineString(lines)) else { panic!() };
    assert_eq!(rings[1].as_ptr() as usize, rings_at);

    // A polygon whose rings are closed already: nothing is added, so nothing moves.
    let mut ring = points;
    ring.push(ring[0]);
    let ring_at = ring.as_ptr() as usize;
    let polygon = Polygon { exterior: ring, interiors: vec![] };
    let Geo::Polygon(polygon) = Geo::from(Geometry::Polygon(polygon)) else { panic!() };
    assert_eq!(polygon.exterior().0.as_ptr() as usize, ring_at);
    let Geometry::Polygon(polygon) = Geometry::from(Geo::Polygon(polygon)) else { panic!() };
    assert_eq!(polygon.exterior.as_ptr() as usize, ring_at);
}

fn points(most: usize) -> impl Strategy<Value = Vec<Point>> {
    vec((-180.0..180.0, -90.0..90.0).prop_map(|(x, y)| Point { x, y }), 0..most)
}

/// A ring that ends where it starts, as every ring `geo-types` gives does.
fn ring() -> impl Strategy<Value = Vec<Point>> {
    points(5).prop_map(|mut points| {
        points.extend(points.first().copied());
        points
    })
}

fn geometry() -> impl Strategy<Value = Geometry> {
    let polygon = || {
        (ring(), vec(ring(), 0..3))
            .prop_map(|(exterior, interiors)| Polygon { exterior, interiors })
    };
    let shape = prop_oneof![
        points(2).prop_map(|points| Geometry::Point(points.first().copied().unwrap_or_default())),
        points(6).prop_map(Geometry::Line),
        polygon().prop_map(Geometry::Polygon),
        points(6).prop_map(Geometry::MultiPoint),
        vec(points(4), 0..3).prop_map(Geometry::MultiLine),
        vec(polygon(), 0..3).prop_map(Geometry::MultiPolygon),
    ];
    shape.prop_recursive(3, 12, 3, |inner| vec(inner, 0..3).prop_map(Geometry::Collection))
}

proptest! {
    #[test]
    fn a_geometry_with_closed_rings_comes_back_from_geo_types(geometry in geometry()) {
        let geo = Geo::from(&geometry);
        prop_assert_eq!(Geometry::from(&geo), geometry.clone());
        // The same from the bytes of a document.
        let bytes = encode(&Value::Geometry(geometry)).unwrap();
        prop_assert_eq!(Doc::new(&bytes).unwrap().root().as_geo(), Some(geo));
    }
}
