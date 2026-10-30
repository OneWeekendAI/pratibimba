//! A1, the advisor: measures each arm's uplift offline and queues suggestions as cards. It never
//! changes live traffic. B6 adds the propensity each estimate weights by.

pub mod propensity;
pub mod uplift;
pub mod pause;
pub mod run;
