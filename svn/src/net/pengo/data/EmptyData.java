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


import java.io.*;

/** 
 * NewFile.. currently immutable.
 */
public class EmptyData extends Data {
    static final byte[] nada = new byte[0];
    
    public long getLength() {
        return 0;
    }
    
    //FIXME: throw out of bounds thing
    public Data getSelection(long start, long length) {
        return this;
    }
    
    public TransparentData subdata(long start, long length) {
        return new TransparentData(this,0,0);
    }
    
    
    //public Chunk getMultpleSelection(...);    
    
    public String toString() {
        return "empty file";
    }
        
    public String getType() {
        return "empty file";
    }

    public InputStream dataStream() {
        return new ByteArrayInputStream(nada);
    }
    
    public InputStream getDataStream(long offset) {
        return new ByteArrayInputStream(nada);
    }

    public InputStream getDataStream(long start, long length) {
        return new ByteArrayInputStream(nada);
    }
}
