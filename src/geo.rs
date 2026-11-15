//! Geometries to and from the types of the `geo` crates.
//!
//! [`geo`](https://docs.rs/geo) computes on the types of
//! [`geo-types`](https://docs.rs/geo-types): areas, lengths, containment, distances, bounding
//! rectangles. It re-exports those types, and SurrealDB holds its geometries in them. This
//! module converts both ways, so that a geometry read out of a document is one `geo` works
//! on, and what `geo` gives is one a document holds:
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use fleece::{Doc, Geometry, Value};
//! use geo_types::{Geometry as Geo, Point, polygon};
//!
//! let square = polygon![(x: 0.0, y: 0.0), (x: 4.0, y: 0.0), (x: 4.0, y: 4.0), (x: 0.0, y: 4.0)];
//! let bytes = fleece::encode(&Value::Geometry(Geometry::from(Geo::Polygon(square.clone()))))?;
//!
//! // The geometry of a document as `geo` takes it. With the crate `geo` in scope,
//! // `stored.unsigned_area()` is 16 and `stored.contains(&Point::new(1.0, 1.0))` holds.
//! let stored: Geo = Doc::new(&bytes)?.root().as_geo().ok_or("not a geometry")?;
//! assert_eq!(stored, Geo::Polygon(square));
//! # let _ = Point::new(1.0, 1.0);
//! # Ok(())
//! # }
//! ```
//!
//! Only `geo-types` is a dependency here; the algorithms come with `geo`, in the
//! application that wants them.
//!
//! # What does not come back the same
//!
//! - `geo-types` closes every ring of a polygon: when the last point of a ring is not its
//!   first, the first is added at the end. A polygon with an open ring comes back with that
//!   point more. Polygons that come from `geo-types`, as SurrealDB's all do, are closed.
//! - `geo-types` has three shapes more than a geometry here: a `Line` becomes a line of
//!   its two points, a `Rect` and a `Triangle` the polygon they are.

use ::geo_types as geo;

use crate::reader::ValueRef;
use crate::value::{Geometry, Point, Polygon};

impl From<Point> for geo::Coord<f64> {
    fn from(point: Point) -> Self {
        geo::Coord { x: point.x, y: point.y }
    }
}

impl From<geo::Coord<f64>> for Point {
    fn from(coord: geo::Coord<f64>) -> Self {
        Point { x: coord.x, y: coord.y }
    }
}

impl From<Point> for geo::Point<f64> {
    fn from(point: Point) -> Self {
        geo::Point(point.into())
    }
}

impl From<geo::Point<f64>> for Point {
    fn from(point: geo::Point<f64>) -> Self {
        point.0.into()
    }
}

fn line(points: &[Point]) -> geo::LineString<f64> {
    geo::LineString(points.iter().map(|point| (*point).into()).collect())
}

fn points(line: &geo::LineString<f64>) -> Vec<Point> {
    line.0.iter().map(|coord| (*coord).into()).collect()
}

impl From<&Polygon> for geo::Polygon<f64> {
    fn from(polygon: &Polygon) -> Self {
        let interiors = polygon.interiors.iter().map(|ring| line(ring)).collect();
        geo::Polygon::new(line(&polygon.exterior), interiors)
    }
}

impl From<&geo::Polygon<f64>> for Polygon {
    fn from(polygon: &geo::Polygon<f64>) -> Self {
        Polygon {
            exterior: points(polygon.exterior()),
            interiors: polygon.interiors().iter().map(points).collect(),
        }
    }
}

impl From<&Geometry> for geo::Geometry<f64> {
    fn from(geometry: &Geometry) -> Self {
        match geometry {
            Geometry::Point(point) => geo::Geometry::Point((*point).into()),
            Geometry::Line(points) => geo::Geometry::LineString(line(points)),
            Geometry::Polygon(polygon) => geo::Geometry::Polygon(polygon.into()),
            Geometry::MultiPoint(points) => geo::Geometry::MultiPoint(geo::MultiPoint(
                points.iter().map(|point| (*point).into()).collect(),
            )),
            Geometry::MultiLine(lines) => geo::Geometry::MultiLineString(geo::MultiLineString(
                lines.iter().map(|points| line(points)).collect(),
            )),
            Geometry::MultiPolygon(polygons) => geo::Geometry::MultiPolygon(geo::MultiPolygon(
                polygons.iter().map(Into::into).collect(),
            )),
            Geometry::Collection(geometries) => geo::Geometry::GeometryCollection(
                geo::GeometryCollection(geometries.iter().map(Into::into).collect()),
            ),
        }
    }
}

/// The points of a line given away: the vector that held them holds them still. A point
/// here and a coordinate there are both two doubles, and the standard library collects a
/// vector into one of items of the same size in place, without a second allocation.
fn into_line(points: Vec<Point>) -> geo::LineString<f64> {
    geo::LineString(points.into_iter().map(Into::into).collect())
}

fn into_points(line: geo::LineString<f64>) -> Vec<Point> {
    line.0.into_iter().map(Into::into).collect()
}

impl From<Polygon> for geo::Polygon<f64> {
    fn from(polygon: Polygon) -> Self {
        let interiors = polygon.interiors.into_iter().map(into_line).collect();
        geo::Polygon::new(into_line(polygon.exterior), interiors)
    }
}

impl From<geo::Polygon<f64>> for Polygon {
    fn from(polygon: geo::Polygon<f64>) -> Self {
        let (exterior, interiors) = polygon.into_inner();
        Polygon {
            exterior: into_points(exterior),
            interiors: interiors.into_iter().map(into_points).collect(),
        }
    }
}

impl From<Geometry> for geo::Geometry<f64> {
    /// The geometry given away: its coordinates are not copied.
    fn from(geometry: Geometry) -> Self {
        match geometry {
            Geometry::Point(point) => geo::Geometry::Point(point.into()),
            Geometry::Line(points) => geo::Geometry::LineString(into_line(points)),
            Geometry::Polygon(polygon) => geo::Geometry::Polygon(polygon.into()),
            Geometry::MultiPoint(points) => geo::Geometry::MultiPoint(geo::MultiPoint(
                points.into_iter().map(Into::into).collect(),
            )),
            Geometry::MultiLine(lines) => geo::Geometry::MultiLineString(geo::MultiLineString(
                lines.into_iter().map(into_line).collect(),
            )),
            Geometry::MultiPolygon(polygons) => geo::Geometry::MultiPolygon(geo::MultiPolygon(
                polygons.into_iter().map(Into::into).collect(),
            )),
            Geometry::Collection(geometries) => geo::Geometry::GeometryCollection(
                geo::GeometryCollection(geometries.into_iter().map(Into::into).collect()),
            ),
        }
    }
}

impl From<&geo::Geometry<f64>> for Geometry {
    fn from(geometry: &geo::Geometry<f64>) -> Self {
        match geometry {
            geo::Geometry::Point(point) => Geometry::Point((*point).into()),
            geo::Geometry::Line(line) => Geometry::Line(vec![line.start.into(), line.end.into()]),
            geo::Geometry::LineString(line) => Geometry::Line(points(line)),
            geo::Geometry::Polygon(polygon) => Geometry::Polygon(polygon.into()),
            geo::Geometry::Rect(rect) => Geometry::Polygon((&rect.to_polygon()).into()),
            geo::Geometry::Triangle(triangle) => Geometry::Polygon((&triangle.to_polygon()).into()),
            geo::Geometry::MultiPoint(points) => {
                Geometry::MultiPoint(points.0.iter().map(|point| (*point).into()).collect())
            }
            geo::Geometry::MultiLineString(lines) => {
                Geometry::MultiLine(lines.0.iter().map(points).collect())
            }
            geo::Geometry::MultiPolygon(polygons) => {
                Geometry::MultiPolygon(polygons.0.iter().map(Into::into).collect())
            }
            geo::Geometry::GeometryCollection(geometries) => {
                Geometry::Collection(geometries.0.iter().map(Into::into).collect())
            }
        }
    }
}

impl From<geo::Geometry<f64>> for Geometry {
    /// The geometry given away: its coordinates are not copied.
    fn from(geometry: geo::Geometry<f64>) -> Self {
        match geometry {
            geo::Geometry::Point(point) => Geometry::Point(point.into()),
            geo::Geometry::Line(line) => Geometry::Line(vec![line.start.into(), line.end.into()]),
            geo::Geometry::LineString(line) => Geometry::Line(into_points(line)),
            geo::Geometry::Polygon(polygon) => Geometry::Polygon(polygon.into()),
            geo::Geometry::Rect(rect) => Geometry::Polygon(rect.to_polygon().into()),
            geo::Geometry::Triangle(triangle) => Geometry::Polygon(triangle.to_polygon().into()),
            geo::Geometry::MultiPoint(points) => {
                Geometry::MultiPoint(points.0.into_iter().map(Into::into).collect())
            }
            geo::Geometry::MultiLineString(lines) => {
                Geometry::MultiLine(lines.0.into_iter().map(into_points).collect())
            }
            geo::Geometry::MultiPolygon(polygons) => {
                Geometry::MultiPolygon(polygons.0.into_iter().map(Into::into).collect())
            }
            geo::Geometry::GeometryCollection(geometries) => {
                Geometry::Collection(geometries.0.into_iter().map(Into::into).collect())
            }
        }
    }
}

impl ValueRef<'_> {
    /// The value as a geometry of `geo-types`, when it is a geometry: the field of a
    /// document as the algorithms of `geo` take it, the document around it not decoded.
    pub fn as_geo(&self) -> Option<geo::Geometry<f64>> {
        self.as_geometry().map(Into::into)
    }
}
