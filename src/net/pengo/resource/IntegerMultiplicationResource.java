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
 * IntegerMultiplicationResource.java
 *
 * Created on 25 September 2004, 05:15
 */

package net.pengo.resource;

import net.pengo.pointer.JavaPointer;

/**
 *
 * @author Peter Halasz
 */
public class IntegerMultiplicationResource extends Resource implements ResourceAlertListener {
    
	// these could also be called coefficients, factors, or Multiplicand + Multiplier
	public final JavaPointer factorXP = new JavaPointer(IntResource.class); // (the number that is multiplied by the multiplier)
	public final JavaPointer factorYP = new JavaPointer(IntResource.class); // 
	
	//fixme: make read only somehow?
	public final JavaPointer valueP = new JavaPointer(IntResource.class); // the divisor of a fraction
	
	/** Creates a new instance of IntegerMultiplicationResource */
	public IntegerMultiplicationResource() {
	    this(new IntPrimativeResource(0),new IntPrimativeResource(1));
	}
	
	/** Creates a new instance of IntegerDivisionResource */
	public IntegerMultiplicationResource(IntResource x, IntResource y) {
	    factorXP.setName("Multiplicand");
	    factorXP.addSink(this);
	    setFactorX(x);
	    factorXP.addAlertListenerer(this);

	    factorYP.setName("Multiplier");
	    factorYP.addSink(this);
	    setFactorY(y);
	    factorYP.addAlertListenerer(this);

	    valueP.setName("Result");
	    valueP.addSink(this);

	    valueChanged(this); // update value
	}

    public Resource[] getSources() {
	return new Resource[]{ factorXP, factorYP, valueP };
    }
    
    public void valueChanged(Resource updated) {
	//fixme: use BigInteger maths... when necessary
	    long x = getFactorX().toLong();
	    long y = getFactorY().toLong();
	    
	    long z = x * y;
	    
	    setValue(new IntPrimativeResource(z));
    }
    
    public void resourceRemoved(Resource removed) {
	//fixme: PANIC?!
    }
    
    public IntResource getValue() {
	return (IntResource)valueP.evaluate();
    }
    
    private void setValue(IntResource value) {
	this.valueP.setValue(value);
    }	
	
    public IntResource getFactorX() {
	return (IntResource)factorXP.evaluate();
    }
    
    public void setFactorX(IntResource x) {
	factorXP.setValue(x);
    }
	
    public IntResource getFactorY() {
	return (IntResource)factorYP.evaluate();
    }
    
    public void setFactorY(IntResource y) {
	factorYP.setValue(y);
    }

}