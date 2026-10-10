//! Headless replay: policy receives observed cells and detector events, never target lists.
#[path="../src/target_search.rs"] mod detector;
use terra_autonomy::*;
use terra_navigation::{Pose,Twist};
use terra_mapping::MapSnapshot;
fn input(m:&MapSnapshot,p:Pose,t:f64,frame:u64)->ArbiterInput<'_>{ArbiterInput{now:t,pose:p,pose_time:t,map:Some(m),map_time:Some(t),map_revision:frame,measured:Twist::default(),healthy:true}}
fn observed(p:Pose)->MapSnapshot{let mut m=MapSnapshot{width:60,height:60,resolution:0.25,origin_x:-7.5,origin_y:-7.5,occupancy:vec![-1;3600]};for y in 0..60{for x in 0..60{let px=m.origin_x+(x as f64+0.5)*m.resolution;let py=m.origin_y+(y as f64+0.5)*m.resolution;if (px-p.x).hypot(py-p.y)<2.8{m.occupancy[y*60+x]=0;}}}m}
#[test]fn seeded_search_completes_without_proposal_approval_and_exports_evidence(){
    let mut a=AutonomyArbiter::default();let mut p=Pose::default();let targets=vec![terra_experiment::Survivor{id:1,x:3.0,y:0.0}];let mut records=vec![];
    a.set_detector_capability(vec!["survivor".into()],"sim-survivor-v1".into(),0.).unwrap();a.set_level(LevelRequest{level:Level::TargetSearch,token:"mode".into()});
    a.start_search(SearchRequest{version:1,run_id:"local".into(),search_id:"seed-42".into(),token:"start".into(),target_class:"survivor".into(),bounds:SearchBounds{min_x:-6.,min_y:-6.,max_x:6.,max_y:6.},time_budget_s:120.},&input(&observed(p),p,0.,0)).unwrap();
    for frame in 1..1201{let t=frame as f64*0.1;let m=observed(p);a.set_detector_capability(vec!["survivor".into()],"sim-survivor-v1".into(),t).unwrap();if let Some(status)=a.search_status(t){if let Some(o)=detector::observation(&m,p,&targets,&status,frame,t){let _=a.accept_target_observation(o,t);}}
        let o=a.step(input(&m,p,t,frame));assert!(o.proposal.is_none());records.push(serde_json::json!({"kind":"control_tick","run_id":"local","rover_id":1,"time":t,"pose":p,"search":o.search,"status":o.status,"selected":o.twist,"events":o.events}));
        if o.search.as_ref().is_some_and(|s|s.phase==SearchPhase::Completed){assert_eq!(o.twist,Twist::default());assert_eq!(o.pending_reports.len(),1);assert!(o.pending_reports[0].frame_ids.len()>=3);if let Ok(path)=std::env::var("L4_REPLAY_OUTPUT"){let data=records.iter().map(|r|serde_json::to_string(r).unwrap()).collect::<Vec<_>>().join("\n");std::fs::write(path,format!("{data}\n")).unwrap();}return;}
        p.yaw+=o.twist.angular*0.1;p.x+=o.twist.linear*p.yaw.cos()*0.1;p.y+=o.twist.linear*p.yaw.sin()*0.1;
        assert!(!o.search.as_ref().is_some_and(|s|s.phase.terminal()),"unexpected terminal {:?} at {t}, pose {p:?}",o.search);
    }panic!("search did not confirm target");
}
#[test]fn wrong_class_does_not_generate_observation(){let m=observed(Pose::default());let r=SearchRequest{version:1,run_id:"local".into(),search_id:"s".into(),token:"t".into(),target_class:"chair".into(),bounds:SearchBounds{min_x:-6.,min_y:-6.,max_x:6.,max_y:6.},time_budget_s:60.};let s=SearchController::start(r,0.).unwrap().status(0.,true);assert!(detector::observation(&m,Pose::default(),&[terra_experiment::Survivor{id:1,x:1.,y:0.}],&s,1,0.).is_none());}
