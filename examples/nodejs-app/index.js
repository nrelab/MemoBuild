const express = require('express');
const app = express();
const port = 3000;

app.get('/', (req, res) => {
    res.send('Hello from MemoBuild cached Node.js app!');
});

app.listen(port, () => {
    console.log(`Example app listening at http://localhost:${port}`);
});
