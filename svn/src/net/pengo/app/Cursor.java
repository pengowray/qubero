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

/*
 * CursorData.java
 *
 * Created on 4 September 2002, 11:40
 */

/**
 *
 * @author  Peter Halasz
 */
package net.pengo.app;
import net.pengo.data.*;

public class Cursor implements Comparable {
    private long startBefore;
    
    /** Creates a new instance of CursorData */
    public Cursor(long startBefore) {
        this.startBefore = startBefore;
    }
    
    /** Compares this object with the specified object for order.  Returns a
     * negative integer, zero, or a positive integer as this object is less
     * than, equal to, or greater than the specified object.<p>
     *
     * @param   o the Object to be compared.
     * @return  a negative integer, zero, or a positive integer as this object
     * 		is less than, equal to, or greater than the specified object.
     *
     * @throws ClassCastException if the specified object's type prevents it
     *         from being compared to this Object.
     *
     *
     */
    public int compareTo(Cursor o) {
        long thisVal = startBefore;
        long anotherVal = o.startBefore;
        return (thisVal<anotherVal ? -1 : (thisVal==anotherVal ? 0 : 1));
    }
    
    public int compareTo(Data o) {
        long thisVal = startBefore;
        long anotherVal = o.getStart();
        return (thisVal <= anotherVal ? -1 : 1);
    }

    
    public int compareTo(Object o) {
        if (o instanceof Cursor) {
            return compareTo((Cursor)o);
        }
        if (o instanceof Data) {
            return compareTo((Data)o);
        }
        System.out.println("comparing a cursor with a..." + o.getClass() + " argh! " + o);
        return compareTo((Data)o);
    }
    
    
    public long getStartBefore() {
        return startBefore;
    }
    
    public String toString() {
        return "Cursor sits before: " + startBefore;
    }
    public boolean equals(Data obj) {
        return false;
    }
    public boolean equals(Cursor obj) {
        return (obj.startBefore == this.startBefore);
    }
    
}
