import { Button, Col, Form, FormControl, FormLabel, Row } from "react-bootstrap";
import { AuthState } from "./websocket";
import { useRef, useState } from "react";

const loginUser = async (username : string, password: string, callback : (state: AuthState) => void) => {
    // Create the login request
    /*const response = await fetch('https://yourapi.com/login', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ username, password }),
    });
  
    // Check if login was successful
    if (response.ok) {
      const data = await response.json();
      const token = data.token;
  
      // Store the token securely in localStorage or a cookie
      localStorage.setItem('jwt', token);
      return token;
    } else {
      throw new Error('Login failed');
    }*/
    callback({token:"token",username:username});
};

export function LoginPage({beginSession}:{beginSession : (state: AuthState) => void}){
    const [username, setUsername] = useState<string>("");
    const [password, setPassword] = useState<string>("");

    return <Row className="justify-content-center align-items-center" style={{height:600}}>
                <Col md={12} className="d-flex justify-content-center">
                    <Form onSubmit={(event) => {
                        event.preventDefault();
                        if(!username || username.length == 0){
                          alert("Enter a non-blank username.");
                        }else{
                          loginUser(username || "", password || "",beginSession);     
                        }        
                    }}>
                        <Form.Text><h3>Login to Automarathon</h3></Form.Text>
                        <FormLabel htmlFor="username">Username</FormLabel>
                        <FormControl type="text" id="username" value={username} onChange={(e)=>setUsername(e.currentTarget.value)}/>
                        <FormLabel htmlFor="password">Password</FormLabel>
                        <FormControl type="password" id="password" value={password} onChange={(e)=>setPassword(e.currentTarget.value)}/>
                        <Button className="mt-3" type="submit" variant="primary">Login</Button>
                    </Form>
                </Col>
            </Row>;
}