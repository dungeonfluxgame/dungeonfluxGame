use super::purpose::QueueClass;
use super::queues::QueueBounds;
use super::scheduler::{BrokerLimits, BrokerLimitsError};

#[test]
fn a_queue_bound_of_zero_is_refused_for_every_class() {
    for class in QueueClass::ALL.iter().copied() {
        assert_eq!(
            QueueBounds::default().with_bound(class, 0),
            Err(BrokerLimitsError::ZeroQueueBound { class })
        );
    }
    assert_eq!(
        QueueBounds::new(1, 1, 0, 1),
        Err(BrokerLimitsError::ZeroQueueBound {
            class: QueueClass::Lookahead
        })
    );
}

#[test]
fn a_bound_set_for_one_class_leaves_the_others_as_they_were() {
    let bounds = QueueBounds::default()
        .with_bound(QueueClass::Plate, 2)
        .unwrap();

    assert_eq!(bounds.of(QueueClass::Plate), 2);
    for class in [
        QueueClass::Live,
        QueueClass::Lookahead,
        QueueClass::Background,
    ] {
        assert_eq!(bounds.of(class), QueueBounds::default().of(class));
    }
    assert_eq!(
        QueueBounds::new(4, 3, 2, 1).unwrap().of(QueueClass::Live),
        4
    );
}

#[test]
fn broker_limits_refuse_a_lane_without_slots_and_a_reserve_that_takes_every_text_slot() {
    use super::purpose::ResourceKind;

    let queues = QueueBounds::default();
    assert_eq!(
        BrokerLimits::new(0, 1, 0, queues),
        Err(BrokerLimitsError::NoSlots {
            kind: ResourceKind::Text
        })
    );
    assert_eq!(
        BrokerLimits::new(2, 0, 1, queues),
        Err(BrokerLimitsError::NoSlots {
            kind: ResourceKind::Image
        })
    );
    assert_eq!(
        BrokerLimits::new(2, 1, 2, queues),
        Err(BrokerLimitsError::ReservedLiveTextSlotsLeaveNone {
            reserved_live_text_slots: 2,
            text_slots: 2
        })
    );
    assert!(BrokerLimits::new(2, 1, 1, queues).is_ok());
    assert_eq!(
        BrokerLimits::default().with_image_slots(0),
        Err(BrokerLimitsError::NoSlots {
            kind: ResourceKind::Image
        })
    );
}

#[test]
fn the_default_broker_limits_pass_their_own_checks() {
    let limits = BrokerLimits::default();

    assert_eq!(
        BrokerLimits::new(
            limits.text_slots,
            limits.image_slots,
            limits.reserved_live_text_slots,
            limits.queues
        ),
        Ok(limits)
    );
}
