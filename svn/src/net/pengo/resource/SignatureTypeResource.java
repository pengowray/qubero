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
 * Created on Jan 25, 2004
 */
package net.pengo.resource;


public class SignatureTypeResource extends Resource {
    private TypeResource result;
    private TypeResource obj;
    private TypeResource[] param;
    
    public SignatureTypeResource(
            TypeResource result,
            TypeResource obj,
            TypeResource[] param) {
        this.result = result;
        this.obj = obj;
        this.param = param;
    }
    
    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }
    
    
    /**
     * @return Returns the obj.
     */
    public TypeResource getObj() {
        return obj;
    }

    /**
     * @return Returns the param.
     */
    public TypeResource[] getParam() {
        return param;
    }

    /**
     * @return Returns the result.
     */
    public TypeResource getResult() {
        return result;
    }

}
