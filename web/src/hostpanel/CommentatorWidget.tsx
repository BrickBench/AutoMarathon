import { Button, Card, CardBody, CardHeader, FormControl, InputGroup, ListGroup, ListGroupItem } from "react-bootstrap";
import { DiscordUser, Person, StreamEntry, StreamHost } from "../websocket";
import { useEffect, useState } from "react";
import { doPost } from "../Api";
import { Floppy, InputCursorText, Save, SaveFill } from "react-bootstrap-icons";
import InputGroupText from "react-bootstrap/esm/InputGroupText";

export function CommentatorWidget({host} : {host : StreamHost}){
    let [commentators, setCommentators] = useState<{[key: string]: DiscordUser}>({...host.discord_users});

    useEffect(()=>{
        setCommentators({...host.discord_users});
    },[host]);

    let discordUsers = Object.entries(commentators);

    return <Card>
        <CardHeader><h5>Commentator Volume</h5></CardHeader>
        <CardBody className='p-0'>
            {discordUsers ? <><ListGroup>
                {discordUsers.map(([key,user]) => {
                    return <ListGroupItem key={key}>
                        <InputGroup>
                            <InputGroupText>{user.username}</InputGroupText>
                            <FormControl type="number" value={user.volume_percent} onChange={(e)=>{
                                let newUsers:{[key: string]: DiscordUser} = {...commentators};
                                newUsers[key] = {...commentators[key], volume_percent: parseInt(e.currentTarget.value)};
                                setCommentators(newUsers);
                            }}/>
                            <Button onClick={()=>{
                                doPost('discord/volume','PUT', { 
                                    user: commentators[key].username,
                                    volume: commentators[key].volume_percent
                                }); 
                            }}><Floppy></Floppy>
                            </Button>
                        </InputGroup>
                    </ListGroupItem>;
                })}
            </ListGroup>
            </>
            : <h4>Assign an event to a stream first</h4>}
        </CardBody>
    </Card>;
}