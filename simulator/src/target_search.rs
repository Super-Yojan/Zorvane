//! Simulator sensor adapter. Only this layer may inspect hidden target placements.
use terra_autonomy::{SearchStatus,TargetObservation};
use terra_mapping::MapSnapshot;
use terra_navigation::Pose;
/// Visibility uses per-target field of view, range and an observed-free occupancy ray.
pub fn visible(map:&MapSnapshot,pose:Pose,x:f64,y:f64)->bool{
    let distance=(x-pose.x).hypot(y-pose.y);
    if !pose.finite()||!x.is_finite()||!y.is_finite()||distance>5.||!map.resolution.is_finite()||map.resolution<=0.||map.width==0||map.height==0||map.occupancy.len()!=map.width as usize*map.height as usize{return false;}
    let bearing=(y-pose.y).atan2(x-pose.x);let angle=(bearing-pose.yaw).sin().atan2((bearing-pose.yaw).cos());
    if distance>0.05&&angle.abs()>std::f64::consts::FRAC_PI_3{return false;}
    let count=(distance/map.resolution).ceil().max(1.) as usize;
    (0..=count).all(|i|{let t=i as f64/count as f64;let a=((pose.x+(x-pose.x)*t-map.origin_x)/map.resolution).floor();let b=((pose.y+(y-pose.y)*t-map.origin_y)/map.resolution).floor();a>=0.&&b>=0.&&a<map.width as f64&&b<map.height as f64&&(0..65).contains(&map.occupancy[b as usize*map.width as usize+a as usize])})
}
pub fn observation(map:&MapSnapshot,pose:Pose,targets:&[terra_experiment::Survivor],status:&SearchStatus,frame:u64,now:f64)->Option<TargetObservation>{
    if status.target_class!="survivor"{return None;}
    let target=targets.iter().filter(|s|visible(map,pose,s.x,s.y)).min_by(|a,b|(a.x-pose.x).hypot(a.y-pose.y).total_cmp(&(b.x-pose.x).hypot(b.y-pose.y)).then(a.id.cmp(&b.id)))?;
    // Quantized sensor estimate, not direct evaluator-coordinate access by the policy.
    Some(TargetObservation{run_id:status.run_id.clone(),search_id:status.search_id.clone(),frame_id:frame,target_class:"survivor".into(),confidence:0.95,world_x:(target.x*20.).round()/20.,world_y:(target.y*20.).round()/20.,received_at:now,evidence_id:format!("sim-frame-{frame}")})
}
#[cfg(test)]mod tests{use super::*;fn map()->MapSnapshot{MapSnapshot{width:40,height:40,resolution:0.25,origin_x:-5.,origin_y:-5.,occupancy:vec![0;1600]}}
#[test]fn range_and_field_of_view_gate_targets(){let m=map();assert!(visible(&m,Pose::default(),2.,0.));assert!(!visible(&m,Pose::default(),-2.,0.));assert!(!visible(&m,Pose::default(),6.,0.));}
#[test]fn occluded_or_unknown_target_is_not_observed(){let mut m=map();m.occupancy[20*40+24]=100;assert!(!visible(&m,Pose::default(),2.,0.));m.occupancy[20*40+24]=-1;assert!(!visible(&m,Pose::default(),2.,0.));}
}
