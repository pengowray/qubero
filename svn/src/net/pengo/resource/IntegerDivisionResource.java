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
 * IntegerDivisionResource.java
 *
 * Created on 21 September 2004, 07:31
 */

package net.pengo.resource;

import net.pengo.pointer.JavaPointer;

/**
 *
 * @author Peter Halasz
 */
public class IntegerDivisionResource extends Resource implements ResourceAlertListener {
    public final JavaPointer numeratorP = new JavaPointer(IntResource.class); // a number to be divided by the denominator
    public final JavaPointer denominatorP = new JavaPointer(IntResource.class); // the divisor of a fraction
    
    //fixme: make read only somehow?
    public final JavaPointer valueP = new JavaPointer(IntResource.class);
    
    public IntegerDivisionResource() {
	this(new IntPrimativeResource(0), new IntPrimativeResource(1));
    }
    
    /** Creates a new instance of IntegerDivisionResource */
    public IntegerDivisionResource(IntResource numerator, IntResource denominator) {
	numeratorP.setName("Numerator");
	numeratorP.addSink(this);
	setNumerator(numerator);
	numeratorP.addAlertListenerer(this);
	
	denominatorP.setName("Denominator");
	denominatorP.addSink(this);
	setDenominator(denominator);
	denominatorP.addAlertListenerer(this);
	
	valueP.setName("Result");
	valueP.addSink(this);
	
	valueChanged(this); // update value
	
    }
    
    public Resource[] getSources() {
	return new Resource[]{ numeratorP, denominatorP, valueP };
    }
    
    public void valueChanged(Resource updated) {
	System.out.println("div's resource updated: " + updated );
	
	try {
	    long x = getNumerator().toLong();
	    long y = getDenominator().toLong();
	    
	    long z = (long)(x / y);
	    
	    setValue(new IntPrimativeResource(z));
	} catch (ArithmeticException e) { // divide by zero
	    //fixme: ???
	    System.out.println("divide by zero.. setting value to 0");
	    setValue(new IntPrimativeResource(0));
	}
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
    
    public IntResource getNumerator() {
	return (IntResource)numeratorP.evaluate();
    }
    
    public void setNumerator(IntResource numerator) {
	this.numeratorP.setValue(numerator);
    }
    
    
    public IntResource getDenominator() {
	return (IntResource)denominatorP.evaluate();
    }
    
    public void setDenominator(IntResource denominator) {
	this.denominatorP.setValue(denominator);
    }
    
}
