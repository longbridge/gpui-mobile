//! Drives `GpuiMapView`, the tile map `GpuiPlatformView` creates for a "map"
//! platform view, by the view's ID.

use super::{LatLng, MapCamera, MapMarker, MapType};
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::{JDoubleArray, JObject, JValue};

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiMapView";

pub fn set_center(view: i64, center: LatLng) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        env.call_static_method(
            &cls,
            jni::jni_str!("setCenter"),
            jni::jni_sig!("(JDD)V"),
            &[
                JValue::Long(view),
                JValue::Double(center.latitude),
                JValue::Double(center.longitude),
            ],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn set_zoom(view: i64, zoom: f64) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        env.call_static_method(
            &cls,
            jni::jni_str!("setZoom"),
            jni::jni_sig!("(JD)V"),
            &[JValue::Long(view), JValue::Double(zoom)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn set_map_type(_view: i64, map_type: MapType) -> Result<(), String> {
    match map_type {
        MapType::Normal => Ok(()),
        // The tile view draws one tile source; pick another with TileMapProvider::new.
        other => Err(format!(
            "tile maps have no {other:?} type: use another tile server"
        )),
    }
}

pub fn add_marker(view: i64, marker: &MapMarker) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let id = env.new_string(&marker.id).e()?;
        let title = match &marker.title {
            Some(title) => JObject::from(env.new_string(title).e()?),
            None => JObject::null(),
        };
        let snippet = match &marker.snippet {
            Some(snippet) => JObject::from(env.new_string(snippet).e()?),
            None => JObject::null(),
        };
        env.call_static_method(
            &cls,
            jni::jni_str!("addMarker"),
            jni::jni_sig!("(JLjava/lang/String;DDLjava/lang/String;Ljava/lang/String;)V"),
            &[
                JValue::Long(view),
                JValue::Object(&id),
                JValue::Double(marker.position.latitude),
                JValue::Double(marker.position.longitude),
                JValue::Object(&title),
                JValue::Object(&snippet),
            ],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn remove_marker(view: i64, marker_id: &str) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let id = env.new_string(marker_id).e()?;
        env.call_static_method(
            &cls,
            jni::jni_str!("removeMarker"),
            jni::jni_sig!("(JLjava/lang/String;)V"),
            &[JValue::Long(view), JValue::Object(&id)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn clear_markers(view: i64) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        env.call_static_method(
            &cls,
            jni::jni_str!("clearMarkers"),
            jni::jni_sig!("(J)V"),
            &[JValue::Long(view)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn camera(view: i64) -> Result<MapCamera, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getCamera"),
                jni::jni_sig!("(J)[D"),
                &[JValue::Long(view)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;
        if result.is_null() {
            return Err("The map is not created yet".into());
        }
        // SAFETY: `getCamera` returns a `double[]`.
        let array = unsafe { JDoubleArray::from_raw(env, result.into_raw()) };
        let mut values = [0.0; 3];
        array.get_region(env, 0, &mut values).e()?;
        Ok(MapCamera {
            center: LatLng {
                latitude: values[0],
                longitude: values[1],
            },
            zoom: values[2],
        })
    })
}

pub fn selected_marker(view: i64) -> Result<Option<String>, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getSelectedMarker"),
                jni::jni_sig!("(J)Ljava/lang/String;"),
                &[JValue::Long(view)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;
        Ok((!result.is_null()).then(|| jni_helpers::get_string(env, &result)))
    })
}
