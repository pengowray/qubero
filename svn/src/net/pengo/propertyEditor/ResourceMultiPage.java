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
 * Created on Apr 9, 2004
 *
 */
package net.pengo.propertyEditor;

import net.pengo.resource.Resource;

/**
 * A MultiPage defined by the Resource it is based on.
 *
 * Allows cursor to be restored (on a rebuilt menu/list).
 *
 * @author Peter Halasz
 *
 */
public class ResourceMultiPage extends MultiPage {
    
    private Resource resource;
    
    /**
     * @param form
     * @param page
     * @param name
     */
    public ResourceMultiPage(PropertiesForm form, Resource res) {
        super(form, (PropertyPage[]) res.getPrimaryPages().toArray(new PropertyPage[0]), res.nameOrType());
        this.resource = res;
        // add a listener to the "nameOrType"
    }
    
    /**
     * Getter for property resource.
     * @return Value of property resource.
     */
    public Resource getResource() {
        return resource;
    }
    
    /**
     * Setter for property resource.
     * @param resource New value of property resource.
     */
    public void setResource(Resource resource) {
        //FIXME: (re)set pages and name
        this.resource = resource;
    }
   
    
}
