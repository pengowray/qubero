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
 * ContainerResource.java
 *
 * Generic container resource. Put anything into a resource.
 * "Double click for details." Created for debugging.
 *
 * @author Peter Halasz
 */

package net.pengo.resource;

public class ContainerResource extends Resource
{
	Object o;
	
	public ContainerResource(Object o) {
		this.o = o;
	}
	
	public void doubleClickAction() {
		super.doubleClickAction();
	    //System.out.println("  " + o.getClass() + " -- " + o);
	}

	public String valueDesc() {
		if (o==null)
			return "[NULL]";
		
		return o.toString();
	}

//	public String getName() {
//	    if (super.getName() == null) {
//	        return o.toString();
//	    }
//	    
//	    return super.getName();
//	}
	
	public String getTypeName() {
		if (o==null)
			return "[NULL container]";
		
	    return shortTypeName(o.getClass());
	}
	
	
	public boolean isPointer() {
	    return true;
	}

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#getSources()
     */
    public Resource[] getSources() {
        return new Resource[]{};
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
}

