/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * IntAddressedResource.java
 *
 * @author  Peter Halasz
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

