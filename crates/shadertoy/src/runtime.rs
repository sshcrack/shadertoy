use crate::ffi::{check, last_error};
use crate::types::{checked_image_len, zeroed_image_vec};
use crate::{Error, HeadlessContext, PassProfileSample, PassTiming, Project, Result, RgbImage};
use shadertoy_sys as sys;
use std::ffi::{CStr, CString};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::NonNull;
use std::rc::Rc;

/// Compiled ShaderToy runtime bound to a caller-owned current OpenGL context.
///
/// The context reference guarantees it outlives all GL objects owned by this runtime.
pub struct Runtime<'context> {
    handle: NonNull<sys::st_runtime>,
    context: &'context HeadlessContext,
    _thread_affine: PhantomData<Rc<()>>,
}

impl<'context> Runtime<'context> {
    pub fn new(context: &'context HeadlessContext) -> Result<Self> {
        context.make_current()?;
        // SAFETY: generated binding calls the repository-owned stable C ABI with a current GL context.
        let handle = unsafe { sys::st_runtime_create() };
        let handle = NonNull::new(handle).ok_or_else(last_error)?;
        Ok(Self {
            handle,
            context,
            _thread_affine: PhantomData,
        })
    }

    pub fn load_project(&mut self, project: &Project) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: both handles are valid; native implementation creates/copies its own ShaderDocument.
        check(unsafe { sys::st_runtime_load_project(self.handle.as_ptr(), project.as_ptr()) })
    }

    pub fn load_sttf(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.context.make_current()?;
        let path = CString::new(path.as_ref().to_string_lossy().as_bytes())?;
        // SAFETY: runtime handle and C string are valid across the call.
        check(unsafe { sys::st_runtime_load_sttf(self.handle.as_ptr(), path.as_ptr()) })
    }

    pub fn save_sttf(&self, path: impl AsRef<Path>) -> Result<()> {
        self.context.make_current()?;
        let path = CString::new(path.as_ref().to_string_lossy().as_bytes())?;
        // SAFETY: runtime handle and C string are valid across the call.
        check(unsafe { sys::st_runtime_save_sttf(self.handle.as_ptr(), path.as_ptr()) })
    }

    pub fn tick(&mut self, frame_rate: f32) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and its context is current.
        unsafe { sys::st_runtime_tick(self.handle.as_ptr(), frame_rate) };
        Ok(())
    }

    pub fn tick_fixed(&mut self, delta_seconds: f32, frame_rate: f32) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and its context is current.
        unsafe { sys::st_runtime_tick_fixed(self.handle.as_ptr(), delta_seconds, frame_rate) };
        Ok(())
    }

    pub fn reset_time(&mut self) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and its context is current.
        unsafe { sys::st_runtime_reset_time(self.handle.as_ptr()) };
        Ok(())
    }

    pub fn set_fixed_state(
        &mut self,
        time_seconds: f32,
        frame: i32,
        frame_rate: f32,
    ) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        unsafe {
            sys::st_runtime_set_fixed_state(self.handle.as_ptr(), time_seconds, frame, frame_rate)
        };
        Ok(())
    }

    pub fn set_replay_state(
        &mut self,
        time_seconds: f32,
        time_delta: f32,
        frame: i32,
        frame_rate: f32,
    ) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and its context is current.
        unsafe {
            sys::st_runtime_set_replay_state(
                self.handle.as_ptr(),
                time_seconds,
                time_delta,
                frame,
                frame_rate,
            )
        };
        Ok(())
    }

    pub fn time(&self) -> f32 {
        // SAFETY: reading the runtime's scalar time does not mutate GL state.
        unsafe { sys::st_runtime_time(self.handle.as_ptr()) }
    }

    pub fn time_delta(&self) -> f32 {
        // SAFETY: reading the runtime's scalar time delta does not mutate GL state.
        unsafe { sys::st_runtime_time_delta(self.handle.as_ptr()) }
    }

    pub fn frame_rate(&self) -> f32 {
        // SAFETY: reading the runtime's scalar frame rate does not mutate GL state.
        unsafe { sys::st_runtime_frame_rate(self.handle.as_ptr()) }
    }

    pub fn frame(&self) -> i32 {
        // SAFETY: reading the runtime's scalar frame counter does not mutate GL state.
        unsafe { sys::st_runtime_frame(self.handle.as_ptr()) }
    }

    pub fn pause(&mut self) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        unsafe { sys::st_runtime_pause(self.handle.as_ptr()) };
        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        unsafe { sys::st_runtime_resume(self.handle.as_ptr()) };
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        // SAFETY: reading the runtime running flag does not mutate GL state.
        unsafe { sys::st_runtime_is_running(self.handle.as_ptr()) != 0 }
    }

    pub fn time_scale(&self) -> f32 {
        // SAFETY: reading the runtime scalar time scale does not mutate GL state.
        unsafe { sys::st_runtime_time_scale(self.handle.as_ptr()) }
    }

    pub fn set_time_scale(&mut self, log2_scale: f32) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        unsafe { sys::st_runtime_set_time_scale(self.handle.as_ptr(), log2_scale) };
        Ok(())
    }

    pub fn set_mouse(&mut self, x: f32, y: f32, down: bool, clicked: bool) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        check(unsafe {
            sys::st_runtime_set_mouse(
                self.handle.as_ptr(),
                x,
                y,
                i32::from(down),
                i32::from(clicked),
            )
        })
    }

    pub fn clear_mouse(&mut self) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        check(unsafe { sys::st_runtime_clear_mouse(self.handle.as_ptr()) })
    }

    pub fn set_key(&mut self, key: u8, down: bool, pressed: bool) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        check(unsafe {
            sys::st_runtime_set_key(
                self.handle.as_ptr(),
                key,
                i32::from(down),
                i32::from(pressed),
            )
        })
    }

    pub fn clear_key_transients(&mut self) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        check(unsafe { sys::st_runtime_clear_key_transients(self.handle.as_ptr()) })
    }

    pub fn set_uniform_f32(&mut self, name: &str, values: &[f32]) -> Result<()> {
        if values.is_empty() || values.len() > 4 {
            return Err(Error::Native(
                "custom float uniforms require 1 to 4 values".into(),
            ));
        }
        self.context.make_current()?;
        let name = CString::new(name)?;
        // SAFETY: runtime/name/value slice remain valid for the duration of the native call.
        check(unsafe {
            sys::st_runtime_set_uniform_f32(
                self.handle.as_ptr(),
                name.as_ptr(),
                values.as_ptr(),
                values.len(),
            )
        })
    }

    pub fn set_uniform_i32(&mut self, name: &str, value: i32) -> Result<()> {
        self.context.make_current()?;
        let name = CString::new(name)?;
        // SAFETY: runtime handle and name are valid for the duration of the native call.
        check(unsafe {
            sys::st_runtime_set_uniform_i32(self.handle.as_ptr(), name.as_ptr(), value)
        })
    }

    pub fn render(&mut self, width: u32, height: u32) -> Result<RgbImage> {
        self.context.make_current()?;
        let mut pixels = zeroed_image_vec::<u8>(width, height, 3)?;
        // SAFETY: runtime and output buffer are valid; native side writes exactly out_len bytes on success.
        check(unsafe {
            sys::st_runtime_render_rgb(
                self.handle.as_ptr(),
                width,
                height,
                pixels.as_mut_ptr(),
                pixels.len(),
            )
        })?;
        Ok(RgbImage::new(width, height, pixels))
    }

    pub fn snapshot_pass_rgb(&mut self, pass: &str, width: u32, height: u32) -> Result<RgbImage> {
        self.context.make_current()?;
        let pass = CString::new(pass)?;
        let mut pixels = zeroed_image_vec::<u8>(width, height, 3)?;
        // SAFETY: runtime/buffer/string are valid across the call.
        check(unsafe {
            sys::st_runtime_snapshot_pass_rgb(
                self.handle.as_ptr(),
                pass.as_ptr(),
                pixels.as_mut_ptr(),
                pixels.len(),
            )
        })?;
        Ok(RgbImage::new(width, height, pixels))
    }

    pub fn snapshot_pass_rgba32f(
        &mut self,
        pass: &str,
        width: u32,
        height: u32,
    ) -> Result<Vec<f32>> {
        self.context.make_current()?;
        let pass = CString::new(pass)?;
        let mut pixels = zeroed_image_vec::<f32>(width, height, 4)?;
        // SAFETY: runtime/buffer/string are valid across the call.
        check(unsafe {
            sys::st_runtime_snapshot_pass_rgba32f(
                self.handle.as_ptr(),
                pass.as_ptr(),
                pixels.as_mut_ptr(),
                pixels.len(),
            )
        })?;
        Ok(pixels)
    }

    pub fn snapshot_pass_output_rgb(
        &mut self,
        pass: &str,
        output: u32,
        width: u32,
        height: u32,
    ) -> Result<RgbImage> {
        self.context.make_current()?;
        let pass = CString::new(pass)?;
        let mut pixels = zeroed_image_vec::<u8>(width, height, 3)?;
        // SAFETY: runtime/buffer/string are valid across the call.
        check(unsafe {
            sys::st_runtime_snapshot_pass_rgb_output(
                self.handle.as_ptr(),
                pass.as_ptr(),
                output,
                pixels.as_mut_ptr(),
                pixels.len(),
            )
        })?;
        Ok(RgbImage::new(width, height, pixels))
    }

    pub fn snapshot_pass_output_rgba32f(
        &mut self,
        pass: &str,
        output: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<f32>> {
        self.context.make_current()?;
        let pass = CString::new(pass)?;
        let mut pixels = zeroed_image_vec::<f32>(width, height, 4)?;
        // SAFETY: runtime/buffer/string are valid across the call.
        check(unsafe {
            sys::st_runtime_snapshot_pass_rgba32f_output(
                self.handle.as_ptr(),
                pass.as_ptr(),
                output,
                pixels.as_mut_ptr(),
                pixels.len(),
            )
        })?;
        Ok(pixels)
    }

    pub fn snapshot_storage_buffer(&mut self, name: &str, size: usize) -> Result<Vec<u8>> {
        self.context.make_current()?;
        let name = CString::new(name)?;
        let mut data = vec![0u8; size];
        // SAFETY: runtime/string/output are valid for the duration of the native call.
        check(unsafe {
            sys::st_runtime_snapshot_storage_buffer(
                self.handle.as_ptr(),
                name.as_ptr(),
                data.as_mut_ptr(),
                data.len(),
            )
        })?;
        Ok(data)
    }

    pub fn restore_storage_buffer(&mut self, name: &str, data: &[u8]) -> Result<()> {
        self.context.make_current()?;
        let name = CString::new(name)?;
        // SAFETY: runtime/string/data are valid for the duration of the native call.
        check(unsafe {
            sys::st_runtime_restore_storage_buffer(
                self.handle.as_ptr(),
                name.as_ptr(),
                data.as_ptr(),
                data.len(),
            )
        })
    }

    pub fn update_texture_rgba8(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<()> {
        if rgba.len() != crate::types::checked_image_len(width, height, 4)? {
            return Err(Error::InvalidRgbaBuffer { width, height });
        }
        self.context.make_current()?;
        let name = CString::new(name)?;
        // SAFETY: runtime/string/data are valid for the duration of the native call.
        check(unsafe {
            sys::st_runtime_update_texture_rgba8(
                self.handle.as_ptr(),
                name.as_ptr(),
                width,
                height,
                rgba.as_ptr(),
                rgba.len(),
            )
        })
    }

    pub fn set_wireframe_pass(&mut self, pass: &str, enabled: bool) -> Result<()> {
        let pass = CString::new(pass)?;
        // SAFETY: runtime handle and C string remain valid across the call.
        check(unsafe {
            sys::st_runtime_set_wireframe_pass(
                self.handle.as_ptr(),
                pass.as_ptr(),
                i32::from(enabled),
            )
        })
    }

    pub fn reload_pass_source(&mut self, pass: &str, source: &str) -> Result<()> {
        self.context.make_current()?;
        let pass = CString::new(pass)?;
        let source = CString::new(source)?;
        // SAFETY: runtime handle and C strings are valid across the call.
        check(unsafe {
            sys::st_runtime_reload_pass_source(self.handle.as_ptr(), pass.as_ptr(), source.as_ptr())
        })
    }

    pub fn set_profiling(&mut self, enabled: bool) -> Result<()> {
        // Preserve the historical library behavior: plain profiling isolates
        // pass completion. The CLI opts into non-intrusive profiling explicitly.
        self.set_profiling_mode(enabled, true)
    }

    pub fn set_profiling_mode(&mut self, enabled: bool, sync_per_pass: bool) -> Result<()> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        unsafe {
            sys::st_runtime_set_profiling_sync_per_pass(
                self.handle.as_ptr(),
                i32::from(sync_per_pass),
            );
            sys::st_runtime_set_profiling(self.handle.as_ptr(), i32::from(enabled));
        }
        Ok(())
    }

    pub fn frame_gpu_nanoseconds(&self) -> Result<u64> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        Ok(unsafe { sys::st_runtime_profile_frame_gpu_nanoseconds(self.handle.as_ptr()) })
    }

    pub fn frame_gpu_timestamp_nanoseconds(&self) -> Result<u64> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid and context is current.
        Ok(
            unsafe {
                sys::st_runtime_profile_frame_gpu_timestamp_nanoseconds(self.handle.as_ptr())
            },
        )
    }

    pub fn pass_timings(&self) -> Result<Vec<PassTiming>> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid; returned count indexes the runtime-owned timing vector.
        let count = unsafe { sys::st_runtime_profile_pass_count(self.handle.as_ptr()) };
        let mut timings = Vec::with_capacity(count);
        for index in 0..count {
            // SAFETY: index is below count from the same timing vector.
            let name_len =
                unsafe { sys::st_runtime_profile_pass_name_len(self.handle.as_ptr(), index) };
            if name_len == 0 {
                return Err(Error::Native("invalid native profiling pass name".into()));
            }
            let mut name = vec![0 as std::ffi::c_char; name_len];
            let mut native = sys::st_pass_timing::default();
            // SAFETY: buffers are correctly sized and valid for the duration of the call.
            check(unsafe {
                sys::st_runtime_profile_pass(
                    self.handle.as_ptr(),
                    index,
                    name.as_mut_ptr(),
                    name.len(),
                    &mut native,
                )
            })?;
            // SAFETY: native API guarantees NUL termination within the provided buffer.
            let name = unsafe { CStr::from_ptr(name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            timings.push(PassTiming {
                name,
                gpu_nanoseconds: native.gpu_nanoseconds,
                width: native.width,
                height: native.height,
            });
        }
        Ok(timings)
    }

    pub fn pass_profile_samples(&self) -> Result<Vec<PassProfileSample>> {
        self.context.make_current()?;
        // SAFETY: runtime handle is valid; returned count indexes the runtime-owned sample vector.
        let count = unsafe { sys::st_runtime_profile_pass_count(self.handle.as_ptr()) };
        let mut samples = Vec::with_capacity(count);
        for index in 0..count {
            // Names use the same pass ordering and lengths as the legacy timing vector.
            let name_len =
                unsafe { sys::st_runtime_profile_pass_name_len(self.handle.as_ptr(), index) };
            if name_len == 0 {
                return Err(Error::Native("invalid native profiling pass name".into()));
            }
            let mut name = vec![0 as std::ffi::c_char; name_len];
            let mut native = sys::st_pass_profile_sample::default();
            // SAFETY: buffers are correctly sized and valid for the duration of the call.
            check(unsafe {
                sys::st_runtime_profile_pass_sample(
                    self.handle.as_ptr(),
                    index,
                    name.as_mut_ptr(),
                    name.len(),
                    &mut native,
                )
            })?;
            // SAFETY: native API guarantees NUL termination within the provided buffer.
            let name = unsafe { CStr::from_ptr(name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            samples.push(PassProfileSample {
                name,
                gpu_execution_nanoseconds: native.gpu_execution_nanoseconds,
                attributed_nanoseconds: native.attributed_nanoseconds,
                completion_wait_nanoseconds: native.completion_wait_nanoseconds,
                width: native.width,
                height: native.height,
                sample_valid: native.sample_valid != 0,
            });
        }
        Ok(samples)
    }

    pub fn override_pass_rgba8(
        &mut self,
        pass: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<()> {
        if rgba.len() != checked_image_len(width, height, 4)? {
            return Err(Error::InvalidRgbaBuffer { width, height });
        }
        self.context.make_current()?;
        let pass = CString::new(pass)?;
        // SAFETY: runtime/data/string are valid across the call; native implementation copies data.
        check(unsafe {
            sys::st_runtime_override_pass_rgba8(
                self.handle.as_ptr(),
                pass.as_ptr(),
                width,
                height,
                rgba.as_ptr(),
                rgba.len(),
            )
        })
    }

    pub fn restore_pass_rgba32f(
        &mut self,
        pass: &str,
        width: u32,
        height: u32,
        rgba: &[f32],
    ) -> Result<()> {
        if rgba.len() != checked_image_len(width, height, 4)? {
            return Err(Error::InvalidRgbaBuffer { width, height });
        }
        self.context.make_current()?;
        let pass = CString::new(pass)?;
        // SAFETY: runtime/data/string are valid across the call; native implementation copies data.
        check(unsafe {
            sys::st_runtime_restore_pass_rgba32f(
                self.handle.as_ptr(),
                pass.as_ptr(),
                width,
                height,
                rgba.as_ptr(),
                rgba.len(),
            )
        })
    }
}

impl Drop for Runtime<'_> {
    fn drop(&mut self) {
        // Keep the context current while native runtime destroys GL resources.
        let _ = self.context.make_current();
        // SAFETY: handle came from st_runtime_create and is owned by self.
        unsafe { sys::st_runtime_destroy(self.handle.as_ptr()) };
    }
}
