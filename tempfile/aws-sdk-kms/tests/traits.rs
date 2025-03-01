/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use aws_sdk_kms as kms;
use kms::operation::create_alias::CreateAliasError;
use kms::operation::generate_random::GenerateRandom;

fn assert_send_sync<T: Send + Sync + 'static>() {}
fn assert_send_fut<T: Send + 'static>(_: T) {}
fn assert_debug<T: std::fmt::Debug>() {}

#[tokio::test]
async fn types_are_send_sync() {
    assert_send_sync::<kms::Error>();
    assert_send_sync::<kms::error::SdkError<CreateAliasError>>();
    assert_send_sync::<kms::operation::create_alias::CreateAliasError>();
    assert_send_sync::<kms::operation::create_alias::CreateAliasOutput>();
    assert_send_sync::<kms::Client>();
    assert_send_sync::<GenerateRandom>();
    let conf = kms::Config::builder().build();
    assert_send_fut(kms::Client::from_conf(conf).list_keys().send());
}

#[tokio::test]
async fn client_is_debug() {
    let conf = kms::Config::builder().build();
    let client = kms::Client::from_conf(conf);
    assert_ne!(format!("{:?}", client), "");
}

#[tokio::test]
async fn client_is_clone() {
    let conf = kms::Config::builder().build();
    let client = kms::Client::from_conf(conf);

    fn is_clone(it: impl Clone) {
        drop(it)
    }

    is_clone(client);
}

#[test]
fn types_are_debug() {
    assert_debug::<kms::Client>();
    assert_debug::<kms::operation::generate_random::builders::GenerateRandomFluentBuilder>();
    assert_debug::<kms::operation::create_alias::builders::CreateAliasFluentBuilder>();
}
