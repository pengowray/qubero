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

package net.pengo.data;


import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.BufferUnderflowException;
import java.nio.ByteBuffer;

/** 
 * rudimentary file editing. entire file is kept in an array.
 * this is NOT editableArrayData
 */
public class ArrayData extends Data {
    private byte[] byteArray;
    String name;
    
    protected ByteBuffer byteBuffer;
    
    public ArrayData() {
        setByteArray(new byte[0]);        
    }

    public ArrayData(byte[] byteArray) {
    	setByteArray(byteArray);
    }	
    
    public ArrayData(byte[] byteArray, String name) {
    	setByteArray(byteArray);
        this.name = name;
    }	

    public InputStream getDataStream(long offset, long length) {
        return new ByteArrayInputStream(byteArray, (int)offset, (int)length); // no loss.
    }

    public InputStream getDataStream(long offset) {
        return new ByteArrayInputStream(byteArray, (int)offset, byteArray.length-(int)offset); // no loss.
    }

    public InputStream dataStream(){
        return new ByteArrayInputStream(byteArray);
    }
    
    public byte[] readByteArray() {
    	//FIXME: should clone?
    	return byteArray;
    }

    public byte[] readByteArray(long start, int length) throws IOException {
    	try {
        	byte[] bytes = new byte[length];
        	
        	//??? always returns from the first byte only
        	//byteBuffer.position((int) start); 
    		//return byteBuffer.get(bytes).array();
        	
        	System.arraycopy(getByteArray(), (int)start, bytes, 0, length);
        	
        	return bytes;
        	
    	} catch (BufferUnderflowException e) {
    		throw new IOException(e.getMessage());
    	}
    }
    
    public long getLength() {
        return (long)byteArray.length;
    }

    public String toString() {
        if (name == null)
            return "byte array data";
            
        return name;
    }
    
	protected byte[] getByteArray() {
		return byteArray;
	}
	
	protected void setByteArray(byte[] byteArray) {
		this.byteArray = byteArray;
		byteBuffer = ByteBuffer.wrap(byteArray);
	}
}
