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
 * Created on Jan 17, 2004
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.resource;

/**
 * @author Peter Halasz
 *
 *
i've been thinking about giving user space programs more awareness of 
"regions".. so a program can create its own region-specific variables.. 
basically global variables that are specific to anything allocated within 
a specific region.. so if you have a large number of objects that would 
otherwise have pointers to the one object you can just stick them all in 
a "region"..

*/
public class Region extends Resource {

    /**
     * 
     */
    public Region() {
        super();
        // TODO Auto-generated constructor stub
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#getSources()
     */
    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }

}
