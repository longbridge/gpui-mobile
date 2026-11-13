//! Embedded maps for Android and iOS.
//!
//! A [`MapView`] is drawn by a [`MapProvider`], which creates the native view
//! and returns a [`MapController`] that drives it. Apps choose the provider:
//!
//! - [`TileMapProvider`] (the default): on Android, `GpuiMapView`, a map drawn
//!   from raster tiles with no SDK and no API key. OpenStreetMap's tiles by
//!   default, or any `{z}/{x}/{y}` tile server with
//!   [`TileMapProvider::new`]. Pan, pinch and double-tap zoom, markers with a
//!   title bubble on tap, attribution shown. OpenStreetMap's tiles are for
//!   light use (<https://operations.osmfoundation.org/policies/tiles/>); apps
//!   with real traffic point it at their own or a commercial tile server. On
//!   iOS the controls are not implemented yet.
//! - Your own: implement [`MapProvider`] and [`MapController`] for any map SDK
//!   (Google Maps, MapLibre, Mapbox…). On Android, register the native view's
//!   type with an [`AndroidPlatformViewFactory`](crate::android::platform_view::AndroidPlatformViewFactory)
//!   in Rust and with `GpuiPlatformView.registerViewType` in Java, create it
//!   through [`PlatformViewRegistry`], and drive it through your own JNI calls
//!   (`GpuiPlatformView.getView(id)` returns the view). Use it per map with
//!   [`MapView::with_provider`], or for every [`MapView::new`] with
//!   [`set_default_provider`].
//!
//! Embed the view with
//! `components::platform_view_element::platform_view_element(map.platform_view_handle()?)`.
//!
//! Unlike `maps_launcher` which opens an external app, this package
//! embeds an interactive map directly in the GPUI render tree.
//!
//! Feature-gated behind `maps`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

use crate::platform_view::{
    PlatformViewBounds, PlatformViewHandle, PlatformViewParams, PlatformViewRegistry,
};
use std::sync::{Arc, RwLock};

/// Geographic coordinate.
#[derive(Debug, Clone, Copy)]
pub struct LatLng {
    pub latitude: f64,
    pub longitude: f64,
}

/// Map type / style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapType {
    /// Standard road map.
    Normal,
    /// Satellite imagery.
    Satellite,
    /// Satellite with road overlays.
    Hybrid,
    /// Terrain/topographic.
    Terrain,
}

/// Configuration for creating a map view.
#[derive(Debug, Clone)]
pub struct MapSettings {
    /// Initial center coordinate.
    pub center: LatLng,
    /// Initial zoom level (0-20, higher = closer).
    pub zoom: f64,
    /// Map type/style.
    pub map_type: MapType,
    /// Enable zoom gestures.
    pub zoom_gestures_enabled: bool,
    /// Enable scroll/pan gestures.
    pub scroll_gestures_enabled: bool,
    /// Enable rotate gestures.
    pub rotate_gestures_enabled: bool,
    /// Show the user's location (requires permission). Not drawn by
    /// [`TileMapProvider`] yet.
    pub my_location_enabled: bool,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            center: LatLng {
                latitude: 37.7749,
                longitude: -122.4194,
            },
            zoom: 12.0,
            map_type: MapType::Normal,
            zoom_gestures_enabled: true,
            scroll_gestures_enabled: true,
            rotate_gestures_enabled: true,
            my_location_enabled: false,
        }
    }
}

/// A map marker/pin.
#[derive(Debug, Clone)]
pub struct MapMarker {
    /// Unique identifier for this marker.
    pub id: String,
    /// Position of the marker.
    pub position: LatLng,
    /// Title shown in the info window.
    pub title: Option<String>,
    /// Snippet/subtitle shown in the info window.
    pub snippet: Option<String>,
}

/// Where the map looks, which changes as the user pans and zooms.
#[derive(Debug, Clone, Copy)]
pub struct MapCamera {
    pub center: LatLng,
    pub zoom: f64,
}

/// Creates map views. Implement it to draw maps with another SDK or tile source.
pub trait MapProvider: Send + Sync {
    /// Create a native map view showing `settings`, and the controller that
    /// drives it. The view is laid out by the element it is embedded in.
    fn create(&self, settings: &MapSettings) -> Result<Box<dyn MapController>, String>;
}

/// Drives one map view made by a [`MapProvider`].
pub trait MapController: Send + Sync {
    /// The platform view to embed with `platform_view_element`.
    fn platform_view(&self) -> Arc<PlatformViewHandle>;

    fn set_center(&self, center: LatLng) -> Result<(), String>;

    fn set_zoom(&self, zoom: f64) -> Result<(), String>;

    /// Providers without other map types return an error for anything but
    /// [`MapType::Normal`].
    fn set_map_type(&self, map_type: MapType) -> Result<(), String> {
        match map_type {
            MapType::Normal => Ok(()),
            other => Err(format!("this map provider has no {other:?} maps")),
        }
    }

    fn add_marker(&self, marker: &MapMarker) -> Result<(), String>;

    fn remove_marker(&self, marker_id: &str) -> Result<(), String>;

    fn clear_markers(&self) -> Result<(), String>;

    /// Where the map looks now, after the user's gestures.
    fn camera(&self) -> Result<MapCamera, String>;

    /// The marker the user selected by tapping it, if the provider tracks it.
    fn selected_marker(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
}

static DEFAULT_PROVIDER: RwLock<Option<Arc<dyn MapProvider>>> = RwLock::new(None);

/// Use `provider` for every [`MapView::new`] from now on.
pub fn set_default_provider(provider: Arc<dyn MapProvider>) {
    *DEFAULT_PROVIDER.write().expect("poisoned") = Some(provider);
}

/// The provider [`MapView::new`] uses: the one given to [`set_default_provider`],
/// else OpenStreetMap tiles.
pub fn default_provider() -> Arc<dyn MapProvider> {
    DEFAULT_PROVIDER
        .read()
        .expect("poisoned")
        .clone()
        .unwrap_or_else(|| Arc::new(TileMapProvider::openstreetmap()))
}

/// Maps drawn from raster tiles by gpui-mobile's own `GpuiMapView` (Android).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileMapProvider {
    url_template: String,
    attribution: String,
}

impl TileMapProvider {
    /// OpenStreetMap's standard tiles, credited as their policy requires.
    pub fn openstreetmap() -> Self {
        Self::new(
            "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
            "© OpenStreetMap contributors",
        )
    }

    /// Tiles from `url_template` (with `{z}`, `{x}` and `{y}`), with
    /// `attribution` drawn in the map's corner (empty for none).
    pub fn new(url_template: impl Into<String>, attribution: impl Into<String>) -> Self {
        Self {
            url_template: url_template.into(),
            attribution: attribution.into(),
        }
    }
}

impl Default for TileMapProvider {
    fn default() -> Self {
        Self::openstreetmap()
    }
}

/// Register the "map" platform view factory.
fn ensure_factory_registered() {
    let registry = PlatformViewRegistry::global();
    if !registry.has_factory("map") {
        #[cfg(target_os = "android")]
        {
            use crate::android::platform_view::AndroidPlatformViewFactory;
            registry.register("map", Box::new(AndroidPlatformViewFactory::new("map")));
        }
        #[cfg(target_os = "ios")]
        {
            use crate::ios::platform_view::IosPlatformViewFactory;
            registry.register("map", Box::new(IosPlatformViewFactory::new("map")));
        }
    }
}

impl MapProvider for TileMapProvider {
    fn create(&self, settings: &MapSettings) -> Result<Box<dyn MapController>, String> {
        if settings.map_type != MapType::Normal {
            return Err(format!(
                "tile maps have no {:?} type: use another tile server",
                settings.map_type
            ));
        }
        ensure_factory_registered();
        let params = [
            ("latitude", settings.center.latitude.to_string()),
            ("longitude", settings.center.longitude.to_string()),
            ("zoom", settings.zoom.to_string()),
            ("zoom_gestures", settings.zoom_gestures_enabled.to_string()),
            (
                "scroll_gestures",
                settings.scroll_gestures_enabled.to_string(),
            ),
            (
                "rotate_gestures",
                settings.rotate_gestures_enabled.to_string(),
            ),
            ("my_location", settings.my_location_enabled.to_string()),
            ("tile_url", self.url_template.clone()),
            ("attribution", self.attribution.clone()),
        ];
        let params = PlatformViewParams {
            bounds: PlatformViewBounds::default(),
            creation_params: params
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        };
        let handle = PlatformViewRegistry::global().create_view("map", params)?;
        Ok(Box::new(TileMapController {
            handle: Arc::new(handle),
        }))
    }
}

struct TileMapController {
    handle: Arc<PlatformViewHandle>,
}

impl TileMapController {
    #[cfg(target_os = "android")]
    fn id(&self) -> i64 {
        self.handle.id().0 as i64
    }
}

/// Runs the Android or iOS implementation of a control, or reports that maps
/// need a mobile platform.
macro_rules! platform {
    ($self:ident, $name:ident $(, $arg:expr)*) => {{
        #[cfg(target_os = "android")]
        {
            android::$name($self.id() $(, $arg)*)
        }
        #[cfg(target_os = "ios")]
        {
            ios::$name($($arg),*)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = ($self $(, $arg)*);
            Err("maps are only available on Android and iOS".to_string())
        }
    }};
}

impl MapController for TileMapController {
    fn platform_view(&self) -> Arc<PlatformViewHandle> {
        self.handle.clone()
    }

    fn set_center(&self, center: LatLng) -> Result<(), String> {
        platform!(self, set_center, center)
    }

    fn set_zoom(&self, zoom: f64) -> Result<(), String> {
        platform!(self, set_zoom, zoom)
    }

    fn set_map_type(&self, map_type: MapType) -> Result<(), String> {
        platform!(self, set_map_type, map_type)
    }

    fn add_marker(&self, marker: &MapMarker) -> Result<(), String> {
        platform!(self, add_marker, marker)
    }

    fn remove_marker(&self, marker_id: &str) -> Result<(), String> {
        platform!(self, remove_marker, marker_id)
    }

    fn clear_markers(&self) -> Result<(), String> {
        platform!(self, clear_markers)
    }

    fn camera(&self) -> Result<MapCamera, String> {
        #[cfg(target_os = "android")]
        {
            android::camera(self.id())
        }
        #[cfg(not(target_os = "android"))]
        {
            Err("reading the map camera is only implemented on Android".into())
        }
    }

    fn selected_marker(&self) -> Result<Option<String>, String> {
        #[cfg(target_os = "android")]
        {
            android::selected_marker(self.id())
        }
        #[cfg(not(target_os = "android"))]
        {
            Ok(None)
        }
    }
}

/// An embedded map view, drawn by a [`MapProvider`].
pub struct MapView {
    controller: Option<Box<dyn MapController>>,
}

impl std::fmt::Debug for MapView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapView")
            .field("disposed", &self.controller.is_none())
            .finish()
    }
}

impl MapView {
    /// Create a map view with the default provider (see [`default_provider`]).
    pub fn new(settings: MapSettings) -> Result<Self, String> {
        Self::with_provider(default_provider().as_ref(), settings)
    }

    /// Create a map view drawn by `provider`.
    pub fn with_provider(
        provider: &dyn MapProvider,
        settings: MapSettings,
    ) -> Result<Self, String> {
        Ok(Self {
            controller: Some(provider.create(&settings)?),
        })
    }

    fn controller(&self) -> Result<&dyn MapController, String> {
        self.controller
            .as_deref()
            .ok_or_else(|| "The map view is disposed".to_string())
    }

    /// The platform view handle for embedding in a GPUI element.
    pub fn platform_view_handle(&self) -> Option<Arc<PlatformViewHandle>> {
        self.controller.as_ref().map(|c| c.platform_view())
    }

    /// The controller, for provider-specific calls after a downcast of your own.
    pub fn map_controller(&self) -> Option<&dyn MapController> {
        self.controller.as_deref()
    }

    /// Where the map looks now, after the user's gestures.
    pub fn camera(&self) -> Result<MapCamera, String> {
        self.controller()?.camera()
    }

    /// The marker whose title bubble the user opened by tapping it.
    pub fn selected_marker(&self) -> Result<Option<String>, String> {
        self.controller()?.selected_marker()
    }

    /// Set the map center coordinate.
    pub fn set_center(&mut self, center: LatLng) -> Result<(), String> {
        self.controller()?.set_center(center)
    }

    /// Set the zoom level.
    pub fn set_zoom(&mut self, zoom: f64) -> Result<(), String> {
        self.controller()?.set_zoom(zoom)
    }

    /// Set the map type/style.
    pub fn set_map_type(&mut self, map_type: MapType) -> Result<(), String> {
        self.controller()?.set_map_type(map_type)
    }

    /// Add a marker to the map.
    pub fn add_marker(&self, marker: &MapMarker) -> Result<(), String> {
        self.controller()?.add_marker(marker)
    }

    /// Remove a marker by ID.
    pub fn remove_marker(&self, marker_id: &str) -> Result<(), String> {
        self.controller()?.remove_marker(marker_id)
    }

    /// Remove all markers from the map.
    pub fn clear_markers(&self) -> Result<(), String> {
        self.controller()?.clear_markers()
    }

    /// Dispose of the map view and release resources.
    pub fn dispose(&mut self) {
        if let Some(controller) = self.controller.take() {
            controller.platform_view().dispose();
        }
    }
}

impl Drop for MapView {
    fn drop(&mut self) {
        self.dispose();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_view::{PlatformView, PlatformViewId};
    use std::sync::Mutex;

    struct NoView(PlatformViewId);

    impl PlatformView for NoView {
        fn id(&self) -> PlatformViewId {
            self.0
        }
        fn view_type(&self) -> &str {
            "test_map"
        }
        fn set_bounds(&self, _: PlatformViewBounds) {}
        fn set_visible(&self, _: bool) {}
        fn set_z_index(&self, _: i32) {}
        fn dispose(&self) {}
        fn is_disposed(&self) -> bool {
            false
        }
    }

    /// A provider that records what the map was asked to do.
    #[derive(Default)]
    struct Recording(Arc<Mutex<Vec<String>>>);

    struct RecordingController {
        view: Arc<PlatformViewHandle>,
        calls: Arc<Mutex<Vec<String>>>,
        camera: Mutex<MapCamera>,
    }

    impl MapProvider for Recording {
        fn create(&self, settings: &MapSettings) -> Result<Box<dyn MapController>, String> {
            self.0
                .lock()
                .unwrap()
                .push(format!("create zoom {}", settings.zoom));
            Ok(Box::new(RecordingController {
                view: Arc::new(PlatformViewHandle::new(Box::new(NoView(
                    PlatformViewId::next(),
                )))),
                calls: self.0.clone(),
                camera: Mutex::new(MapCamera {
                    center: settings.center,
                    zoom: settings.zoom,
                }),
            }))
        }
    }

    impl MapController for RecordingController {
        fn platform_view(&self) -> Arc<PlatformViewHandle> {
            self.view.clone()
        }
        fn set_center(&self, center: LatLng) -> Result<(), String> {
            self.camera.lock().unwrap().center = center;
            Ok(())
        }
        fn set_zoom(&self, zoom: f64) -> Result<(), String> {
            self.camera.lock().unwrap().zoom = zoom;
            self.calls.lock().unwrap().push(format!("zoom {zoom}"));
            Ok(())
        }
        fn add_marker(&self, marker: &MapMarker) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("marker {}", marker.id));
            Ok(())
        }
        fn remove_marker(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn clear_markers(&self) -> Result<(), String> {
            Ok(())
        }
        fn camera(&self) -> Result<MapCamera, String> {
            Ok(*self.camera.lock().unwrap())
        }
    }

    #[test]
    fn map_view_drives_the_provider_it_was_given() {
        let provider = Recording::default();
        let calls = provider.0.clone();
        let mut map = MapView::with_provider(
            &provider,
            MapSettings {
                zoom: 3.0,
                ..Default::default()
            },
        )
        .unwrap();
        map.set_zoom(5.0).unwrap();
        map.add_marker(&MapMarker {
            id: "a".into(),
            position: LatLng {
                latitude: 1.0,
                longitude: 2.0,
            },
            title: None,
            snippet: None,
        })
        .unwrap();
        assert_eq!(map.camera().unwrap().zoom, 5.0);
        // The trait's defaults: no other map types, no selection.
        assert!(map.set_map_type(MapType::Satellite).is_err());
        assert_eq!(map.selected_marker().unwrap(), None);
        assert_eq!(
            *calls.lock().unwrap(),
            ["create zoom 3", "zoom 5", "marker a"]
        );

        map.dispose();
        assert!(map.camera().is_err());
    }

    #[test]
    fn set_default_provider_replaces_openstreetmap() {
        let provider = Recording::default();
        let calls = provider.0.clone();
        set_default_provider(Arc::new(provider));
        let map = MapView::new(MapSettings::default()).unwrap();
        assert!(map.platform_view_handle().is_some());
        assert_eq!(calls.lock().unwrap().len(), 1);
        *DEFAULT_PROVIDER.write().unwrap() = None;
    }
}
