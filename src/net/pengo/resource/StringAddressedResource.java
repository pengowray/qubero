/**
 * IntAddressedResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.io.IOException;

import net.pengo.data.ArrayData;
import net.pengo.data.Data;
import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;

public class StringAddressedResource extends StringResource implements AddressedResource {
    private SmartPointer selResP = new JavaPointer("net.pengo.resource.SelectionResource"); //private SelectionResource selRes;
    //private SmartPointer allowResizeP  = new JavaPointer("net.pengo.resource.BooleanResource");
    //encoding!
    
    public boolean isPrimative() {
        return false;
    }
    
    public StringAddressedResource(SelectionResource selRes) {
        super();
        
        selResP.addSink(this);
        selResP.setName("Selection");
        setSelectionResource(selRes);
    }
    
    public Resource[] getSources() {
        return new Resource[]{selResP};
    }
    
    //fixme: does this make the above redundant? this is dodgy.
    public JavaPointer[] getJPointers() {
        return new JavaPointer[]{(JavaPointer)selResP};
    }
    
    public SelectionResource getSelectionResource() {
        return (SelectionResource)selResP.evaluate();
    }
    
    public void setSelectionResource(SelectionResource selRes) {
        selResP.setValue(selRes);
    }
    
    public void doubleClickAction() {
        // duplicated in BooleanAddrssdResource
        super.doubleClickAction();
        getSelectionResource().makeActive();
    }
    
    public String getValue() {
        //byte[] data = sel.getDataStreamAsArray();
        
        try {
            byte[] data = getSelectionResource().getSelectionData().readByteArray();
            return new String(data); //FIXME: encoding!
        }
        //catch (CloneNotSupportedException e) {}
        catch (IOException e) {
            //FIXME:
            e.printStackTrace();
            return "";
        }
        //return null; // unreachable
    }
    
    /** notification that the underlying binary has been updated */
    public void binaryUpdated() { // (BinaryUpdateEvent e) {
        //FIXME: ignore for now!
    }
    
    /** request to updated underlying binary */
    public void updateBinary(Byte[] b) { 
        //FIXME: needs to be done.
    }
    
    /** notify binary listeners that the value (and thus the binary) has changed */
    private void alertBinaryListener(Data newdata) throws NumberFormatException {
        SelectionResource sel = getSelectionResource();
        sel.insertReplaceResize(newdata);
    }
    
    public void setValue(String value) throws NumberFormatException {
        byte[] b = value.getBytes(); //FIXME: choose encoding
        Data newdata = new ArrayData(b);
        alertBinaryListener(newdata);
    }

    
    
}

