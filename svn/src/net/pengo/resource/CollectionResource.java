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
 * ListResource.java
 *
 * @author Peter Halasz
 */

package net.pengo.resource;

import java.util.Collection;

// a resource that has sub resources
public class CollectionResource extends Resource
{
    Collection resourceList;
    
    public CollectionResource(Collection resourceList) {
        this.resourceList = resourceList;
    }
        
    public Collection getCollection() {
        return resourceList;
    }

//    public String toString() {
//        return resourceList.toString();
//    }
	
    public String valueDesc() {
        return resourceList.size() + " item(s)";
    }
    
    public String getName() {
        if (super.getName() == null) {
            return resourceList.toString();
        }
        
        return super.getName();
    }
    
    public boolean isPointer() {
        return true;
    }


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

