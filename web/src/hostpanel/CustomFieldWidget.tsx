import { useEffect, useState } from "react";
import { doPost } from "../Api";
import { CustomFields } from "../websocket";
import { Button, FormControl, InputGroup } from "react-bootstrap";

function CustomFieldInput({customTemp,key2,setCustomTemp}: {customTemp: any,key2:any,setCustomTemp:any}){
    const [inputState,setInputState]  = useState(customTemp[key2]);
    useEffect(() => {
        setInputState(customTemp[key2]);
    }, [customTemp]);
    return <>
      <label>{key2}</label>
      <input type="text" name={key2} className="form-control" onChange={({ target }) => {
          setInputState(target.value);
          let temp = customTemp;
          temp[key2] = target.value;
          setCustomTemp(temp);
        }} value={inputState || ''}/>
    </>
    ;
}

export function CustomFieldWidget({customFields} : {customFields : CustomFields}){
  const [customTemp,setCustomTemp] = useState(customFields);
  const [newField,setNewField] = useState("");

  useEffect(() => {
    setCustomTemp(customFields);
}, [customFields])

  return (
    <div className="card">
      <div className="card-header">Custom Fields</div>
      <div className="card-body">
        <ul className="list-group list-group-flush">       
            {Object.entries(customTemp).sort().map(([key,val])=>{
              const keystore = key;
              return (<li key={key} className="list-group-item">
                <CustomFieldInput key2={key} customTemp={customTemp} setCustomTemp={setCustomTemp}></CustomFieldInput>
              </li>
                )
            })}
            <li className="list-group-item">
                <InputGroup>
                    <FormControl value={newField} onChange={(e)=>{
                        setNewField(e.currentTarget.value);
                    }}>
                    </FormControl>
                    <Button variant={"success"} onClick={()=>{
                        if(newField && newField.length > 0){
                            doPost('custom-field','PUT',{key:newField,value:""});
                        }else{
                            alert("Enter nonempty field name");
                        }
                    }}>
                        Add New Field
                    </Button>
                </InputGroup>
            </li>
        </ul>
        <button className="btn btn-primary" onClick={() => {
            Object.entries(customTemp).forEach(([key,val])=>{doPost('custom-field','PUT',{key:key,value:val});});
      }}>Save Changes</button>
      </div>
    </div>
  );
}