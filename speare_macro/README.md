## DEPRECATED

# speare_macro

This crate's sole purpose is to reduce the amount of boilerplate needed when working with the `speare` crate.

With it, we can write

```rs
pub struct Foo;

#[process]
impl Foo {
    #[handler]
    async fn bar(&mut self, msg: u64, ctx: &Ctx<Self>) -> Result<String, ()> {
        Ok(format!("{msg}"))
    }

    #[handler]
    async fn baz(&mut self, msg: String, ctx: &Ctx<Self>) -> Result<String, ()> {
        Ok(msg)
    }
}
```

instead of

```rs
pub struct Foo;

impl Process for Foo {
    type Error = ();
}

#[async_trait]
impl Handler<u64> for Foo {
    type Ok = String;
    type Err = ();

    async fn handle(&mut self, msg: u64, ctx: &Ctx<Self>) -> Result<String, ()> {
        Ok(format!("{msg}"))
    }
}

#[async_trait]
impl Handler<String> for Foo {
    type Ok = String;
    type Err = ();

    async fn handle(&mut self, msg: u64, ctx: &Ctx<Self>) -> Result<String, ()> {
        Ok(msg)
    }
}
```