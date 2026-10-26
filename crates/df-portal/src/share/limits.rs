//! Anonymous lookup limits retain only /24 or /48 prefixes for one minute, never viewer history.
use std::{collections::BTreeMap,net::IpAddr};
const MAXIMUM_PREFIXES:usize=4096;
#[derive(Clone,Copy)]
pub struct PublicSharePolicy {global_per_minute:u32,prefix_per_minute:u32}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct InvalidPublicSharePolicy;
impl PublicSharePolicy {
    pub fn new(global_per_minute:u32,prefix_per_minute:u32)->Result<Self,InvalidPublicSharePolicy>{
        if prefix_per_minute==0 || prefix_per_minute>global_per_minute || global_per_minute>60_000 {return Err(InvalidPublicSharePolicy);}
        Ok(Self{global_per_minute,prefix_per_minute})
    }
}
impl Default for PublicSharePolicy{fn default()->Self{Self{global_per_minute:600,prefix_per_minute:60}}}
#[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord)]
enum Prefix{V4([u8;3]),V6([u8;6])}
fn prefix(ip:IpAddr)->Prefix {
    match ip {
        IpAddr::V4(ip)=>{let b=ip.octets();Prefix::V4([b[0],b[1],b[2]])},
        IpAddr::V6(ip)=>{if let Some(ip)=ip.to_ipv4_mapped(){return prefix(IpAddr::V4(ip));}let b=ip.octets();Prefix::V6([b[0],b[1],b[2],b[3],b[4],b[5]])},
    }
}
pub(super) struct PublicLimits {policy:PublicSharePolicy,global:(u64,u32),prefixes:BTreeMap<Prefix,(u64,u32)>}
fn count(bucket:&mut(u64,u32),now:u64,maximum:u32)->bool{
    if now.saturating_sub(bucket.0)>=60_000 {*bucket=(now,0);}
    if bucket.1>=maximum{return false;}bucket.1+=1;true
}
impl PublicLimits{
    pub(super) fn new(policy:PublicSharePolicy)->Self{Self{policy,global:(0,0),prefixes:BTreeMap::new()}}
    pub(super) fn admit(&mut self,ip:Option<IpAddr>,now:u64)->bool{
        if !count(&mut self.global,now,self.policy.global_per_minute){return false;}
        let Some(ip)=ip else{return true;};
        self.prefixes.retain(|_,w|now.saturating_sub(w.0)<60_000);
        let key=prefix(ip);if self.prefixes.len()>=MAXIMUM_PREFIXES&&!self.prefixes.contains_key(&key){return false;}
        count(self.prefixes.entry(key).or_insert((now,0)),now,self.policy.prefix_per_minute)
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn share_lookup_limits_group_ip_prefixes_and_expire_their_buckets(){
        let mut limits=PublicLimits::new(PublicSharePolicy::new(100,2).unwrap());
        assert!(limits.admit(Some("192.0.2.1".parse().unwrap()),0));assert!(limits.admit(Some("192.0.2.2".parse().unwrap()),0));assert!(!limits.admit(Some("192.0.2.3".parse().unwrap()),0));
        assert!(limits.admit(Some("192.0.3.1".parse().unwrap()),0));assert!(limits.admit(Some("192.0.2.3".parse().unwrap()),60_000));
    }
}
