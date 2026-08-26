# Translation parity ledger

Generated, not written by hand — regenerate with `tools/parity.py` after
any change to either side. It answers one question: **which of the
original's runtime functions this port has a counterpart for**, and which
it has never mentioned.

The point is coverage rather than findings. Chasing a symptom can only
find code that is *wrong*; it can never find code that is *absent*,
because a function nobody translated produces no failure to point at —
only a table that feels off. Every row here is either accounted for or it
is not.

A name being present is evidence somebody has been there, **not** that the
translation is right. Verifying a row means reading both sides and marking
it.

Editor, serialisation, mesh generation and UI entry points are filtered
out: this is a player and does not have them.

| | |
|---|---|
| runtime functions in the files this port claims | 322 |
| named somewhere in the port | 209 (64%) |
| never mentioned | 113 |

## PhysicsEngine.cpp — 4/17

Never mentioned in the port:

- [ ] `PhysicsEngine::ReleaseVHO` (PhysicsEngine.cpp:88)
- [ ] `PhysicsEngine::AddCollider` (PhysicsEngine.cpp:134)
- [ ] `PhysicsEngine::RemoveCollider` (PhysicsEngine.cpp:151)
- [ ] `PhysicsEngine::CollectColliders` (PhysicsEngine.cpp:161)
- [ ] `PhysicsEngine::AddCabinetBoundingHitShapes` (PhysicsEngine.cpp:170)
- [ ] `PhysicsEngine::RecordContact` (PhysicsEngine.cpp:217)
- [ ] `PhysicsEngine::GetUIQuadTree` (PhysicsEngine.cpp:230)
- [ ] `PhysicsEngine::GetUIHitObjects` (PhysicsEngine.cpp:237)
- [ ] `PhysicsEngine::RayCast` (PhysicsEngine.cpp:242)
- [ ] `PhysicsEngine::ResetPerFrameStats` (PhysicsEngine.cpp:267)
- [ ] `PhysicsEngine::OnFinishFrame` (PhysicsEngine.cpp:288)
- [ ] `PhysicsEngine::StartPhysics` (PhysicsEngine.cpp:293)
- [ ] `PhysicsEngine::GetPerfInfo` (PhysicsEngine.cpp:740)

## bumper.cpp — 5/7

Never mentioned in the port:

- [ ] `Bumper::UpdateSkirt` (bumper.cpp:414)
- [ ] `Bumper::PlayHit` (bumper.cpp:1124)

## collide.cpp — 18/18

Every runtime function has a counterpart by name.

## collideex.cpp — 29/29

Every runtime function has a counterpart by name.

## core.c — 13/59

Never mentioned in the port:

- [ ] `OnStateChange` (core.c:69)
- [ ] `vp_getSolMask64` (core.c:70)
- [ ] `vp_getDip` (core.c:71)
- [ ] `vp_setDIP` (core.c:72)
- [ ] `saturatedByte` (core.c:112)
- [ ] `drawChar` (core.c:122)
- [ ] `core_initDisplaySize` (core.c:123)
- [ ] `TRAFO_AA` (core.c:255)
- [ ] `core_dmd_render_internal` (core.c:984)
- [ ] `core_dmd_send_libpinmame` (core.c:1038)
- [ ] `core_dmd_send_vpm` (core.c:1060)
- [ ] `core_dmd_send_dmddevice` (core.c:1164)
- [ ] `core_dmd_capture_frame` (core.c:1178)
- [ ] `core_dmd_video_update` (core.c:1248)
- [ ] `core_seg_send_dmddevice` (core.c:1542)
- [ ] `core_seg_render_dmd` (core.c:1554)
- [ ] `core_display_video_update` (core.c:1601)
- [ ] `updateDisplay` (core.c:1615)
- [ ] `core_textOut` (core.c:1895)
- [ ] `core_setLamp` (core.c:2092)
- [ ] `core_setLampBlank` (core.c:2099)
- [ ] `core_setSw` (core.c:2136)
- [ ] `core_updInvSw` (core.c:2155)
- [ ] `core_getPulsedSol` (core.c:2230)
- [ ] `core_getAllSol` (core.c:2242)
- [ ] `core_getAllPhysicSols` (core.c:2286)
- [ ] `core_getDip` (core.c:2349)
- [ ] `core_findSize` (core.c:2689)
- [ ] `core_nvram` (core.c:2743)
- [ ] `core_update_pwm_output_nop` (core.c:2797)
- [ ] `core_update_pwm_output_pulse` (core.c:2802)
- [ ] `core_update_pwm_output_custom` (core.c:2808)
- [ ] `core_update_pwm_output_sol_2_state` (core.c:2819)
- [ ] `cube` (core.c:2852)
- [ ] `core_update_pwm_output_led` (core.c:2990)
- [ ] `core_set_pwm_output_bulb` (core.c:3250)
- [ ] `core_set_pwm_output_led_vfd` (core.c:3265)
- [ ] `core_set_pwm_output_types` (core.c:3275)
- [ ] `core_update_pwm_outputs` (core.c:3286)
- [ ] `core_write_pwm_output` (core.c:3331)
- [ ] `core_write_pwm_output_8b` (core.c:3346)
- [ ] `core_write_masked_pwm_output_8b` (core.c:3363)
- [ ] `core_write_pwm_output_lamp_matrix` (core.c:3381)
- [ ] `core_sound_throttle_adj` (core.c:3913)
- [ ] `core_get_dmd_data` (core.c:3961)
- [ ] `machine_add_timer` (core.c:3978)

## flipper.cpp — 8/10

Never mentioned in the port:

- [ ] `Flipper::UpdatePhysicsSettings` (flipper.cpp:193)
- [ ] `Flipper::SetVertices` (flipper.cpp:272)

## gate.cpp — 8/11

Never mentioned in the port:

- [ ] `Gate::SetGateType` (gate.cpp:33)
- [ ] `Gate::GetOpenAngle` (gate.cpp:145)
- [ ] `Gate::GetCloseAngle` (gate.cpp:169)

## hitball.cpp — 14/15

Never mentioned in the port:

- [ ] `HitBall::GetOldPosition` (hitball.cpp:34)

## hitflipper.cpp — 10/22

Never mentioned in the port:

- [ ] `HitFlipper::UpdatePhysicsFromFlipper` (hitflipper.cpp:131)
- [ ] `ClampDegrees` (hitflipper.cpp:141)
- [ ] `FlipperMoverObject::SetStartAngle` (hitflipper.cpp:253)
- [ ] `FlipperMoverObject::SetEndAngle` (hitflipper.cpp:265)
- [ ] `FlipperMoverObject::GetReturnRatio` (hitflipper.cpp:277)
- [ ] `FlipperMoverObject::GetStrength` (hitflipper.cpp:282)
- [ ] `FlipperMoverObject::GetMass` (hitflipper.cpp:287)
- [ ] `FlipperMoverObject::SetMass` (hitflipper.cpp:292)
- [ ] `FlipperMoverObject::SetSolenoidState` (hitflipper.cpp:429)
- [ ] `FlipperMoverObject::GetStrokeRatio` (hitflipper.cpp:438)
- [ ] `FlipperMoverObject::GetHitTime` (hitflipper.cpp:471)
- [ ] `HitFlipper::HitTestFlipperEnd` (hitflipper.cpp:536)

## hitplunger.cpp — 8/9

Never mentioned in the port:

- [ ] `PlungerMoverObject::SetObjects` (hitplunger.cpp:92)

## hittarget.cpp — 9/11

Never mentioned in the port:

- [ ] `HitTarget::SetMeshType` (hittarget.cpp:39)
- [ ] `HitTarget::TransformVertices` (hittarget.cpp:423)

## kicker.cpp — 17/17

Every runtime function has a counterpart by name.

## quadtree.cpp — 8/17

Never mentioned in the port:

- [ ] `HitQuadtree::Finalize` (quadtree.cpp:183)
- [ ] `HitQuadtree::AllocFourNodes` (quadtree.cpp:199)
- [ ] `HitQuadtree::InitSseArrays` (quadtree.cpp:217)
- [ ] `HitQuadtreeNode::CreateNextLevel` (quadtree.cpp:264)
- [ ] `HitQuadtreeNode::HitTestXRay` (quadtree.cpp:617)
- [ ] `HitQuadtreeNode::DumpTree` (quadtree.cpp:660)
- [ ] `HitQuadtree::HitTestXRay` (quadtree.cpp:686)
- [ ] `EmbreeBoundsFuncBalls` (quadtree.cpp:691)
- [ ] `EmbreeCollideBalls` (quadtree.cpp:709)

## ramp.cpp — 13/20

Never mentioned in the port:

- [ ] `Ramp::AssignHeightToControlPoint` (ramp.cpp:309)
- [ ] `Ramp::AddJoint` (ramp.cpp:766)
- [ ] `Ramp::AddJoint2D` (ramp.cpp:771)
- [ ] `Ramp::AddWallLineSeg` (ramp.cpp:776)
- [ ] `Ramp::IsHabitrail` (ramp.cpp:1008)
- [ ] `Ramp::PrepareHabitrail` (ramp.cpp:1186)
- [ ] `Ramp::GenerateVertexBuffer` (ramp.cpp:2307)

## rubber.cpp — 14/15

Never mentioned in the port:

- [ ] `Rubber::GetCentralCurve` (rubber.cpp:457)

## s11.c — 2/6

Never mentioned in the port:

- [ ] `s11_irqline` (s11.c:101)
- [ ] `s11_piaMainIrq` (s11.c:121)
- [ ] `s11_sw2m` (s11.c:817)
- [ ] `s11_m2sw` (s11.c:820)

## spinner.cpp — 5/10

Never mentioned in the port:

- [ ] `Spinner::GetAngleMax` (spinner.cpp:39)
- [ ] `Spinner::SetAngleMax` (spinner.cpp:41)
- [ ] `Spinner::GetAngleMin` (spinner.cpp:66)
- [ ] `Spinner::SetAngleMin` (spinner.cpp:68)
- [ ] `Spinner::UpdatePlate` (spinner.cpp:400)

## surface.cpp — 11/13

Never mentioned in the port:

- [ ] `Surface::InitTarget` (surface.cpp:80)
- [ ] `Surface::PlaySlingshotHit` (surface.cpp:1737)

## trigger.cpp — 13/16

Never mentioned in the port:

- [ ] `Trigger::InitShape` (trigger.cpp:113)
- [ ] `Trigger::GetPointCenter` (trigger.cpp:780)
- [ ] `Trigger::PutPointCenter` (trigger.cpp:785)
