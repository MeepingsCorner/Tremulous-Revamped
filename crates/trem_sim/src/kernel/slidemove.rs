//! Kernel `slidemove` — port of game/bg_slidemove.c.

use super::bg;
use super::pmove::{Pmove, OVERCLIP, STEPSIZE};
use super::qmath::*;
use super::types::*;
use crate::gen::enums::Event;

const MAX_CLIP_PLANES: usize = 5;

impl<'a, W: PmWorld> Pmove<'a, W> {
    /// `PM_SlideMove`: returns true if the velocity was clipped.
    pub fn pm_slide_move(&mut self, gravity: bool) -> bool {
        let numbumps = 4;
        let mut primal_velocity = self.ps.velocity;
        // C leaves endVelocity uninitialised when !gravity; it is then only ever
        // clipped and discarded, so any value is equivalent.
        let mut end_velocity = [0.0f32; 3];
        if gravity {
            end_velocity = self.ps.velocity;
            end_velocity[2] -= self.ps.gravity as f32 * self.pml.frametime;
            self.ps.velocity[2] = ((self.ps.velocity[2] + end_velocity[2]) as f64 * 0.5) as f32;
            primal_velocity[2] = end_velocity[2];
            if self.pml.ground_plane {
                self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &self.pml.ground_trace.plane.normal, OVERCLIP);
            }
        }
        let mut time_left = self.pml.frametime;
        let mut planes = [[0.0f32; 3]; MAX_CLIP_PLANES];
        let mut numplanes;
        if self.pml.ground_plane {
            numplanes = 1;
            planes[0] = self.pml.ground_trace.plane.normal;
        } else {
            numplanes = 0;
        }
        normalize2(&self.ps.velocity, &mut planes[numplanes]);
        numplanes += 1;

        let mut bumpcount = 0;
        while bumpcount < numbumps {
            let end = ma(&self.ps.origin, time_left, &self.ps.velocity);
            let trace = self.world.trace(&self.ps.origin, Some(&self.mins), Some(&self.maxs), &end, self.ps.client_num, self.tracemask);
            if trace.allsolid {
                self.ps.velocity[2] = 0.0;
                return true;
            }
            if trace.fraction > 0.0 {
                self.ps.origin = trace.endpos;
            }
            if trace.fraction == 1.0 {
                break;
            }
            self.pm_add_touch_ent(trace.entity_num);
            time_left -= time_left * trace.fraction;
            if numplanes >= MAX_CLIP_PLANES {
                self.ps.velocity = [0.0; 3];
                return true;
            }
            let mut same = false;
            for p in planes.iter().take(numplanes) {
                if (dot(&trace.plane.normal, p) as f64) > 0.99 {
                    self.ps.velocity = add(&trace.plane.normal, &self.ps.velocity);
                    same = true;
                    break;
                }
            }
            if same {
                bumpcount += 1;
                continue;
            }
            planes[numplanes] = trace.plane.normal;
            numplanes += 1;

            for i in 0..numplanes {
                let into = dot(&self.ps.velocity, &planes[i]);
                if (into as f64) >= 0.1 {
                    continue;
                }
                if -into > self.pml.impact_speed {
                    self.pml.impact_speed = -into;
                }
                let mut clip_velocity = Self::clip_velocity(&self.ps.velocity, &planes[i], OVERCLIP);
                let mut end_clip_velocity = Self::clip_velocity(&end_velocity, &planes[i], OVERCLIP);
                for j in 0..numplanes {
                    if j == i {
                        continue;
                    }
                    if (dot(&clip_velocity, &planes[j]) as f64) >= 0.1 {
                        continue;
                    }
                    clip_velocity = Self::clip_velocity(&clip_velocity, &planes[j], OVERCLIP);
                    end_clip_velocity = Self::clip_velocity(&end_clip_velocity, &planes[j], OVERCLIP);
                    if dot(&clip_velocity, &planes[i]) >= 0.0 {
                        continue;
                    }
                    let mut dir = cross(&planes[i], &planes[j]);
                    normalize(&mut dir);
                    let d = dot(&dir, &self.ps.velocity);
                    clip_velocity = scale(&dir, d);
                    let mut dir = cross(&planes[i], &planes[j]);
                    normalize(&mut dir);
                    let d = dot(&dir, &end_velocity);
                    end_clip_velocity = scale(&dir, d);
                    for kk in 0..numplanes {
                        if kk == i || kk == j {
                            continue;
                        }
                        if (dot(&clip_velocity, &planes[kk]) as f64) >= 0.1 {
                            continue;
                        }
                        self.ps.velocity = [0.0; 3];
                        return true;
                    }
                }
                self.ps.velocity = clip_velocity;
                end_velocity = end_clip_velocity;
                break;
            }
            bumpcount += 1;
        }
        if gravity {
            self.ps.velocity = end_velocity;
        }
        if self.ps.pm_time != 0 {
            self.ps.velocity = primal_velocity;
        }
        bumpcount != 0
    }

    /// `PM_StepEvent`.
    pub fn pm_step_event(&mut self, from: &Vec3, to: &Vec3, normal: &Vec3) {
        let delta = sub(from, to);
        let mut d_normal = delta;
        normalize(&mut d_normal);
        let mut size = dot(normal, &d_normal) * length(&delta);
        let ev = if size > 0.0 {
            if size > 2.0 {
                Some(if size < 7.0 {
                    Event::Stepdn4
                } else if size < 11.0 {
                    Event::Stepdn8
                } else if size < 15.0 {
                    Event::Stepdn12
                } else {
                    Event::Stepdn16
                })
            } else {
                None
            }
        } else {
            size = (size as f64).abs() as f32;
            if size > 2.0 {
                Some(if size < 7.0 {
                    Event::Step4
                } else if size < 11.0 {
                    Event::Step8
                } else if size < 15.0 {
                    Event::Step12
                } else {
                    Event::Step16
                })
            } else {
                None
            }
        };
        if let Some(e) = ev {
            self.pm_add_event(e);
        }
    }

    /// `PM_StepSlideMove`.
    pub fn pm_step_slide_move(&mut self, gravity: bool, predictive: bool) -> bool {
        let mut stepped = false;
        let normal = bg::client_normal(self.ps);
        let start_o = self.ps.origin;
        let start_v = self.ps.velocity;
        if !self.pm_slide_move(gravity) {
            let down = ma(&start_o, -STEPSIZE as f32, &normal);
            let trace = self.world.trace(&start_o, Some(&self.mins), Some(&self.maxs), &down, self.ps.client_num, self.tracemask);
            if trace.fraction > 0.01 && trace.fraction < 1.0 && !trace.allsolid && self.pml.ground_plane {
                stepped = true;
            }
        } else {
            let down = ma(&start_o, -STEPSIZE as f32, &normal);
            let trace = self.world.trace(&start_o, Some(&self.mins), Some(&self.maxs), &down, self.ps.client_num, self.tracemask);
            if dot(&trace.plane.normal, &self.ps.velocity) > 0.0 && (trace.fraction == 1.0 || dot(&trace.plane.normal, &normal) < 0.7) {
                return stepped;
            }
            let up = ma(&start_o, STEPSIZE as f32, &normal);
            let trace = self.world.trace(&start_o, Some(&self.mins), Some(&self.maxs), &up, self.ps.client_num, self.tracemask);
            if trace.allsolid {
                return stepped;
            }
            let step_v = sub(&trace.endpos, &start_o);
            let mut step_v_normal = step_v;
            normalize(&mut step_v_normal);
            let step_size = dot(&normal, &step_v_normal) * length(&step_v);
            self.ps.origin = trace.endpos;
            self.ps.velocity = start_v;
            if !self.pm_slide_move(gravity) {
                stepped = true;
            }
            let down = ma(&self.ps.origin, -step_size, &normal);
            let trace = self.world.trace(&self.ps.origin, Some(&self.mins), Some(&self.maxs), &down, self.ps.client_num, self.tracemask);
            if !trace.allsolid {
                self.ps.origin = trace.endpos;
            }
            if trace.fraction < 1.0 {
                self.ps.velocity = Self::clip_velocity(&self.ps.velocity, &trace.plane.normal, OVERCLIP);
            }
        }
        if !predictive && stepped {
            self.pm_step_event(&start_o, &self.ps.origin.clone(), &normal);
        }
        stepped
    }

    /// `PM_PredictStepMove`.
    pub fn pm_predict_step_move(&mut self) -> bool {
        let velocity = self.ps.velocity;
        let origin = self.ps.origin;
        let impact_speed = self.pml.impact_speed;
        let stepped = self.pm_step_slide_move(false, true);
        self.ps.velocity = velocity;
        self.ps.origin = origin;
        self.pml.impact_speed = impact_speed;
        stepped
    }
}
